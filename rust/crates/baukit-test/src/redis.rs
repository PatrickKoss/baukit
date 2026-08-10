use std::fmt;

use testcontainers::{ContainerAsync, ImageExt as _, runners::AsyncRunner};
use testcontainers_modules::redis::{REDIS_PORT, Redis};

/// Image tag matching the Redis version pinned across the deploy surface.
const REDIS_IMAGE_TAG: &str = "8.10.0-alpine";

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
}
