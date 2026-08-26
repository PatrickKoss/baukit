use std::{
    fmt, process,
    time::{Duration, SystemTime},
};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt as _,
    core::{ExecCommand, IntoContainerPort as _, WaitFor},
    runners::AsyncRunner,
};
use testcontainers_modules::redis::{REDIS_PORT, Redis};

/// Image tag matching the Redis version pinned across the deploy surface.
const REDIS_IMAGE_TAG: &str = "8.10.0-alpine";

/// Port that Redis Sentinel listens on inside its container.
const REDIS_SENTINEL_PORT: u16 = 26379;

/// Master name used by the disposable Sentinel topology.
const REDIS_SENTINEL_MASTER_NAME: &str = "mymaster";

/// Sentinel configuration path inside the container.
const SENTINEL_CONFIG_PATH: &str = "/tmp/sentinel.conf";

/// Maximum time the Sentinel readiness and failover polls wait for.
///
/// Sentinel enters TILT mode when its event loop is starved, which happens on a
/// host busy starting many containers at once. TILT lasts a fixed 30 seconds and
/// Sentinel performs no failover while in it, so any budget at or below 30 seconds
/// fails outright whenever TILT triggers. This allows for one full TILT period plus
/// the failover that follows it.
const SENTINEL_READY_TIMEOUT: Duration = Duration::from_secs(90);

/// Delay between Sentinel readiness polls.
const SENTINEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// A running disposable Redis container and its connection URL.
///
/// Keep this value alive for as long as Redis is in use. Dropping it invokes
/// Testcontainers' container cleanup behavior.
pub struct RedisTestContainer {
    connection_url: String,
    container: ContainerAsync<Redis>,
}

impl RedisTestContainer {
    /// Returns the host-accessible Redis connection URL.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Returns the underlying Testcontainers guard for advanced test setup.
    #[must_use]
    pub const fn container(&self) -> &ContainerAsync<Redis> {
        &self.container
    }

    /// Splits the fixture into an owned URL and its lifetime guard.
    #[must_use]
    pub fn into_parts(self) -> (String, ContainerAsync<Redis>) {
        (self.connection_url, self.container)
    }
}

impl fmt::Debug for RedisTestContainer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisTestContainer")
            .field("connection_url", &self.connection_url)
            .field("container", &self.container)
            .finish()
    }
}

/// A disposable Redis master, replica, and Sentinel topology.
///
/// The three containers share a Testcontainers-managed bridge network. Sentinel
/// monitors and announces the nodes by their bridge addresses, which are directly
/// routable from the local Linux Docker host; only Sentinel itself needs a mapped
/// host port. Keep this handle alive while the topology is in use. Stopping the
/// master with [`Self::stop_master`] lets Sentinel promote the replica.
pub struct RedisSentinelTestContainer {
    connection_url: String,
    sentinel: ContainerAsync<GenericImage>,
    replica: ContainerAsync<Redis>,
    master: ContainerAsync<Redis>,
}

impl RedisSentinelTestContainer {
    /// Returns the host-accessible `redis+sentinel://` connection URL.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Stops the master immediately so Sentinel can promote the replica.
    ///
    /// The topology is fully settled before this returns, because
    /// [`start_redis_sentinel`] waits for Sentinel to discover the replica. Killing
    /// the master before that discovery leaves Sentinel with no promotion candidate
    /// and no failover happens at all.
    pub async fn stop_master(&self) -> Result<(), RedisTestError> {
        self.master.stop_with_timeout(Some(0)).await?;
        Ok(())
    }

    /// Waits until Sentinel reports the master healthy and has found the replica.
    ///
    /// A fixed sleep cannot express this. Sentinel needs the replica in its own view
    /// before it can promote one, and how long that takes depends on how loaded the
    /// host is. Polling the state Sentinel actually publishes removes the guess.
    async fn wait_until_ready(&self) -> Result<(), RedisTestError> {
        self.poll_until(
            |master| {
                sentinel_field(master, "num-slaves").is_some_and(|slaves| slaves != "0")
                    && sentinel_field(master, "flags").is_some_and(|flags| flags == "master")
            },
            "Sentinel did not report a healthy master with a discovered replica",
        )
        .await
    }

    /// Waits until Sentinel publishes a master other than `previous_address`.
    ///
    /// Sentinel keeps serving the old address for a moment after the replica has
    /// already promoted itself, so a client that resolves in that window connects to
    /// the dead node. Tests that assert post-failover behavior should await this
    /// first.
    pub async fn wait_for_failover(
        &self,
        previous_address: &str,
    ) -> Result<String, RedisTestError> {
        let previous = previous_address.to_owned();
        self.poll_until(
            move |master| {
                sentinel_field(master, "ip").is_some_and(|ip| ip != previous)
                    && sentinel_field(master, "flags").is_some_and(|flags| flags == "master")
            },
            "Sentinel did not publish a promoted master",
        )
        .await?;
        self.master_address().await
    }

    /// Returns the `ip:port` Sentinel currently publishes for the monitored master.
    pub async fn master_address(&self) -> Result<String, RedisTestError> {
        let master = self.sentinel_master_state().await?;
        let ip = sentinel_field(&master, "ip")
            .ok_or_else(|| RedisTestError::Topology("Sentinel reported no master ip".to_owned()))?;
        let port = sentinel_field(&master, "port").ok_or_else(|| {
            RedisTestError::Topology("Sentinel reported no master port".to_owned())
        })?;
        Ok(format!("{ip}:{port}"))
    }

    async fn poll_until<F>(&self, ready: F, failure: &str) -> Result<(), RedisTestError>
    where
        F: Fn(&[String]) -> bool,
    {
        self.poll_until_within(&ready, SENTINEL_READY_TIMEOUT)
            .await
            .map_err(|error| RedisTestError::Topology(format!("{failure}: {error}")))
    }

    async fn poll_until_within<F>(&self, ready: &F, budget: Duration) -> Result<(), RedisTestError>
    where
        F: Fn(&[String]) -> bool,
    {
        let deadline = tokio::time::Instant::now() + budget;
        let mut last_state;
        loop {
            match self.sentinel_master_state().await {
                Ok(master) => {
                    if ready(&master) {
                        return Ok(());
                    }
                    last_state = master.join(" ");
                }
                Err(error) => last_state = error.to_string(),
            }

            if tokio::time::Instant::now() >= deadline {
                let tilt = self.sentinel_tilt_report().await;
                return Err(RedisTestError::Topology(format!(
                    "not reached within {budget:?}; last state: {last_state}{tilt}"
                )));
            }
            tokio::time::sleep(SENTINEL_POLL_INTERVAL).await;
        }
    }

    /// Reports whether Sentinel logged TILT mode, which explains a stalled failover.
    async fn sentinel_tilt_report(&self) -> String {
        let Ok(mut result) = self
            .sentinel
            .exec(ExecCommand::new([
                "redis-cli",
                "-p",
                &REDIS_SENTINEL_PORT.to_string(),
                "info",
                "sentinel",
            ]))
            .await
        else {
            return String::new();
        };
        match result.stdout_to_vec().await {
            Ok(stdout) => {
                let info = String::from_utf8_lossy(&stdout);
                if info.contains("sentinel_tilt:1") {
                    " (Sentinel is in TILT mode: its event loop was starved)".to_owned()
                } else {
                    String::new()
                }
            }
            Err(_) => String::new(),
        }
    }

    async fn sentinel_master_state(&self) -> Result<Vec<String>, RedisTestError> {
        let mut result = self
            .sentinel
            .exec(ExecCommand::new([
                "redis-cli",
                "-p",
                &REDIS_SENTINEL_PORT.to_string(),
                "sentinel",
                "master",
                REDIS_SENTINEL_MASTER_NAME,
            ]))
            .await?;
        let stdout = result.stdout_to_vec().await?;
        Ok(String::from_utf8_lossy(&stdout)
            .lines()
            .map(|line| line.trim().to_owned())
            .collect())
    }

    /// Returns the underlying master container guard.
    #[must_use]
    pub const fn master_container(&self) -> &ContainerAsync<Redis> {
        &self.master
    }

    /// Returns the underlying replica container guard.
    #[must_use]
    pub const fn replica_container(&self) -> &ContainerAsync<Redis> {
        &self.replica
    }

    /// Returns the underlying Sentinel container guard.
    #[must_use]
    pub const fn sentinel_container(&self) -> &ContainerAsync<GenericImage> {
        &self.sentinel
    }
}

impl fmt::Debug for RedisSentinelTestContainer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RedisSentinelTestContainer")
            .field("connection_url", &self.connection_url)
            .field("sentinel", &self.sentinel)
            .field("replica", &self.replica)
            .field("master", &self.master)
            .finish()
    }
}

/// Failure while starting a Redis fixture.
#[derive(Debug, thiserror::Error)]
pub enum RedisTestError {
    /// Testcontainers could not start or inspect the Redis container.
    #[error("could not start Redis test container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// Reading Sentinel's published state failed.
    #[error("could not read Sentinel state: {0}")]
    Io(#[from] std::io::Error),
    /// The Sentinel topology did not reach the expected state in time.
    #[error("{0}")]
    Topology(String),
}

/// Reads one value from Sentinel's flat `field, value` reply.
fn sentinel_field<'a>(reply: &'a [String], field: &str) -> Option<&'a str> {
    reply
        .iter()
        .position(|line| line == field)
        .and_then(|index| reply.get(index + 1))
        .map(String::as_str)
}

/// Starts a disposable Redis container asynchronously.
///
/// Docker is contacted only when this function is called.
pub async fn start_redis() -> Result<RedisTestContainer, RedisTestError> {
    let container = Redis::default().with_tag(REDIS_IMAGE_TAG).start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(REDIS_PORT).await?;
    let connection_url = format!("redis://{host}:{port}/");
    Ok(RedisTestContainer {
        connection_url,
        container,
    })
}

/// Starts a disposable Redis Sentinel topology asynchronously.
///
/// The fixture uses Redis `8.10.0-alpine` for one master, one replica, and one
/// quorum-one Sentinel named `mymaster`. Its low failure-detection interval is
/// intended for failover tests, not production configuration. The local Docker
/// host must be able to route Testcontainers bridge addresses because those are
/// the node addresses Sentinel returns during master discovery.
pub async fn start_redis_sentinel() -> Result<RedisSentinelTestContainer, RedisTestError> {
    let suffix = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let network = format!("baukit-redis-sentinel-{}-{suffix}", process::id());

    let master = Redis::default()
        .with_tag(REDIS_IMAGE_TAG)
        .with_cmd([
            "redis-server",
            "--save",
            "",
            "--appendonly",
            "no",
            "--protected-mode",
            "no",
        ])
        .with_network(&network)
        .start()
        .await?;
    let master_ip = master.get_bridge_ip_address().await?;

    let replica = Redis::default()
        .with_tag(REDIS_IMAGE_TAG)
        .with_cmd([
            "redis-server".to_owned(),
            "--save".to_owned(),
            String::new(),
            "--appendonly".to_owned(),
            "no".to_owned(),
            "--protected-mode".to_owned(),
            "no".to_owned(),
            "--replicaof".to_owned(),
            master_ip.to_string(),
            REDIS_PORT.to_string(),
        ])
        .with_network(&network)
        .start()
        .await?;

    let sentinel_configuration = format!(
        "port {REDIS_SENTINEL_PORT}\n\
         bind 0.0.0.0\n\
         protected-mode no\n\
         dir /tmp\n\
         sentinel monitor {REDIS_SENTINEL_MASTER_NAME} {master_ip} {REDIS_PORT} 1\n\
         sentinel down-after-milliseconds {REDIS_SENTINEL_MASTER_NAME} 500\n\
         sentinel failover-timeout {REDIS_SENTINEL_MASTER_NAME} 3000\n\
         sentinel parallel-syncs {REDIS_SENTINEL_MASTER_NAME} 1\n"
    );
    let sentinel = GenericImage::new("redis", REDIS_IMAGE_TAG)
        .with_exposed_port(REDIS_SENTINEL_PORT.tcp())
        .with_wait_for(WaitFor::message_on_either_std("Running mode=sentinel"))
        .with_copy_to(SENTINEL_CONFIG_PATH, sentinel_configuration.into_bytes())
        .with_cmd(["redis-sentinel", SENTINEL_CONFIG_PATH])
        .with_network(&network)
        .start()
        .await?;
    let host = sentinel.get_host().await?;
    let port = sentinel.get_host_port_ipv4(REDIS_SENTINEL_PORT).await?;
    let connection_url = format!("redis+sentinel://{host}:{port}/{REDIS_SENTINEL_MASTER_NAME}");

    let fixture = RedisSentinelTestContainer {
        connection_url,
        sentinel,
        replica,
        master,
    };
    fixture.wait_until_ready().await?;
    Ok(fixture)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the Redis image"]
    async fn starts_redis_container() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = start_redis().await?;
        assert!(fixture.connection_url().starts_with("redis://"));
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the Redis image"]
    async fn starts_redis_sentinel_topology() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = start_redis_sentinel().await?;
        assert!(fixture.connection_url().starts_with("redis+sentinel://"));
        assert!(fixture.master_container().is_running().await?);
        assert!(fixture.replica_container().is_running().await?);
        assert!(fixture.sentinel_container().is_running().await?);
        Ok(())
    }
}
