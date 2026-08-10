use std::{fmt, process, time::SystemTime};

use testcontainers::{
    ContainerAsync, GenericImage, ImageExt as _,
    core::{IntoContainerPort as _, WaitFor},
    runners::AsyncRunner,
};
use testcontainers_modules::redis::{REDIS_PORT, Redis};

/// Image tag matching the Redis version pinned across the deploy surface.
const REDIS_IMAGE_TAG: &str = "8.10.0-alpine";

/// Port that Redis Sentinel listens on inside its container.
const REDIS_SENTINEL_PORT: u16 = 26379;

/// Master name used by the disposable Sentinel topology.
const REDIS_SENTINEL_MASTER_NAME: &str = "mymaster";

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
    pub async fn stop_master(&self) -> Result<(), RedisTestError> {
        self.master.stop_with_timeout(Some(0)).await?;
        Ok(())
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
        .with_copy_to("/tmp/sentinel.conf", sentinel_configuration.into_bytes())
        .with_cmd(["redis-sentinel", "/tmp/sentinel.conf"])
        .with_network(&network)
        .start()
        .await?;
    let host = sentinel.get_host().await?;
    let port = sentinel.get_host_port_ipv4(REDIS_SENTINEL_PORT).await?;
    let connection_url = format!("redis+sentinel://{host}:{port}/{REDIS_SENTINEL_MASTER_NAME}");
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    Ok(RedisSentinelTestContainer {
        connection_url,
        sentinel,
        replica,
        master,
    })
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
