use std::{fmt, net::TcpListener, time::Duration};

use testcontainers::{ContainerAsync, GenericImage, core::ExecCommand};

/// Maximum time a restarted dependency gets to accept work again.
const READY_TIMEOUT: Duration = Duration::from_secs(60);

/// Delay between readiness probes after a restart.
const READY_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Seconds Docker waits before it kills a stopped container: zero kills at once.
const STOP_IMMEDIATELY: i32 = 0;

/// Failure while injecting or clearing a container fault.
#[derive(Debug, thiserror::Error)]
pub enum FaultError {
    /// Docker refused the stop, start, pause or unpause request.
    #[error("could not change the container state: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// Reading a readiness probe's output failed.
    #[error("could not read the readiness probe: {0}")]
    Io(#[from] std::io::Error),
    /// Docker published the restarted container on another host port.
    #[error(
        "the container came back on host port {after} instead of {before}; start the fixture with a stable host port"
    )]
    PortChanged {
        /// Host port before the restart.
        before: u16,
        /// Host port after the restart.
        after: u16,
    },
    /// The restarted dependency did not pass its readiness probe in time.
    #[error("the dependency did not become ready within {0:?} after the restart")]
    NotReady(Duration),
}

/// Reserves a free host port for a container that must keep its address across restarts.
///
/// Docker assigns a new ephemeral port when a container with a dynamic mapping restarts, so
/// a client holding the old URL could never reconnect.
pub(crate) fn reserve_host_port() -> std::io::Result<u16> {
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(listener.local_addr()?.port())
}

struct ContainerFault<'a> {
    container: &'a ContainerAsync<GenericImage>,
    container_port: u16,
    probe: Vec<String>,
}

impl ContainerFault<'_> {
    async fn stop(&self) -> Result<(), FaultError> {
        self.container
            .stop_with_timeout(Some(STOP_IMMEDIATELY))
            .await?;
        Ok(())
    }

    async fn start(&self) -> Result<(), FaultError> {
        let before = self
            .container
            .get_host_port_ipv4(self.container_port)
            .await?;
        self.container.start().await?;
        let after = self
            .container
            .get_host_port_ipv4(self.container_port)
            .await?;
        if before != after {
            return Err(FaultError::PortChanged { before, after });
        }
        self.wait_until_ready().await
    }

    async fn pause(&self) -> Result<(), FaultError> {
        self.container.pause().await?;
        Ok(())
    }

    async fn resume(&self) -> Result<(), FaultError> {
        self.container.unpause().await?;
        Ok(())
    }

    async fn wait_until_ready(&self) -> Result<(), FaultError> {
        let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
        while tokio::time::Instant::now() < deadline {
            if self.probe_passes().await {
                return Ok(());
            }
            tokio::time::sleep(READY_POLL_INTERVAL).await;
        }
        Err(FaultError::NotReady(READY_TIMEOUT))
    }

    async fn probe_passes(&self) -> bool {
        let Ok(mut result) = self
            .container
            .exec(ExecCommand::new(self.probe.clone()))
            .await
        else {
            return false;
        };
        if result.stdout_to_vec().await.is_err() {
            return false;
        }
        matches!(result.exit_code().await, Ok(Some(0)))
    }
}

impl fmt::Debug for ContainerFault<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ContainerFault")
            .field("container", &self.container.id())
            .field("container_port", &self.container_port)
            .finish_non_exhaustive()
    }
}

macro_rules! dependency_fault {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Debug)]
        pub struct $name<'a> {
            inner: ContainerFault<'a>,
        }

        impl $name<'_> {
            /// Kills the container at once, like a crashed or unreachable dependency.
            ///
            /// # Errors
            ///
            /// Returns [`FaultError::Container`] when Docker cannot stop it.
            pub async fn stop(&self) -> Result<(), FaultError> {
                self.inner.stop().await
            }

            /// Starts a stopped container again with its data and waits until it is ready.
            ///
            /// Clients keep their URL only when the fixture was started with a stable host
            /// port.
            ///
            /// # Errors
            ///
            /// Returns [`FaultError::PortChanged`] when Docker moved the host port,
            /// [`FaultError::NotReady`] when the readiness probe keeps failing, and
            /// [`FaultError::Container`] when Docker cannot start it.
            pub async fn start(&self) -> Result<(), FaultError> {
                self.inner.start().await
            }

            /// Freezes every process in the container. Open connections stay up but get no
            /// answers, like a dependency stuck under load or behind a network partition.
            ///
            /// # Errors
            ///
            /// Returns [`FaultError::Container`] when Docker cannot pause it.
            pub async fn pause(&self) -> Result<(), FaultError> {
                self.inner.pause().await
            }

            /// Unfreezes a paused container.
            ///
            /// # Errors
            ///
            /// Returns [`FaultError::Container`] when Docker cannot unpause it.
            pub async fn resume(&self) -> Result<(), FaultError> {
                self.inner.resume().await
            }
        }
    };
}

dependency_fault!(
    /// Stops, restarts, pauses and resumes a PostgreSQL fixture container.
    ///
    /// Get one from [`crate::PostgresTestContainer::fault`].
    PostgresFault
);

dependency_fault!(
    /// Stops, restarts, pauses and resumes a Redis fixture container.
    ///
    /// Get one from [`crate::RedisTestContainer::fault`] or
    /// [`crate::RedisSentinelTestContainer::master_fault`].
    RedisFault
);

impl<'a> PostgresFault<'a> {
    pub(crate) fn new(container: &'a ContainerAsync<GenericImage>, container_port: u16) -> Self {
        let probe = ["pg_isready", "-h", "127.0.0.1", "-U", "postgres"]
            .map(str::to_owned)
            .to_vec();
        Self {
            inner: ContainerFault {
                container,
                container_port,
                probe,
            },
        }
    }
}

impl<'a> RedisFault<'a> {
    pub(crate) fn new(container: &'a ContainerAsync<GenericImage>, container_port: u16) -> Self {
        let probe = vec![
            "redis-cli".to_owned(),
            "-e".to_owned(),
            "-p".to_owned(),
            container_port.to_string(),
            "ping".to_owned(),
        ];
        Self {
            inner: ContainerFault {
                container,
                container_port,
                probe,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::reserve_host_port;

    #[test]
    fn reserved_port_is_free_again_after_reservation() {
        let port = reserve_host_port().expect("free port");
        assert_ne!(port, 0);
        std::net::TcpListener::bind(("127.0.0.1", port)).expect("reserved port is released");
    }
}
