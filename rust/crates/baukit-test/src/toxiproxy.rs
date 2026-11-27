use std::{
    fmt,
    sync::atomic::{AtomicU16, Ordering},
    time::Duration,
};

use serde_json::{Value, json};
use testcontainers::{
    ContainerAsync, GenericImage, Image, ImageExt as _,
    core::{IntoContainerPort as _, WaitFor},
};

use crate::StartContainer as _;

const TOXIPROXY_IMAGE_NAME: &str = "ghcr.io/shopify/toxiproxy";
const TOXIPROXY_IMAGE_TAG: &str =
    "2.13.0@sha256:5d3dae09bc185dcc5ff6cf43abf6062a4a720aff0c4306a471e773acdcd107aa";
const TOXIPROXY_API_PORT: u16 = 8474;

/// First container port handed to a proxy listener.
const FIRST_PROXY_PORT: u16 = 20_000;

/// Number of proxy listeners one Toxiproxy fixture publishes.
const PROXY_PORT_COUNT: u16 = 8;

/// Toxicity that applies a toxic to every connection.
const ALWAYS: f64 = 1.0;

/// Failure while starting Toxiproxy or changing its proxies.
#[derive(Debug, thiserror::Error)]
pub enum ToxiproxyError {
    /// Testcontainers could not start or inspect a container.
    #[error("could not start or inspect the Toxiproxy container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// The API request did not complete.
    #[error("Toxiproxy API request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The API answered with an error status.
    #[error("Toxiproxy API answered {status}: {body}")]
    Api {
        /// HTTP status code.
        status: u16,
        /// Response body.
        body: String,
    },
    /// Every published proxy port is in use.
    #[error("the fixture publishes {PROXY_PORT_COUNT} proxy ports and all are in use")]
    NoFreeProxyPort,
    /// A toxic duration does not fit the API's millisecond integer.
    #[error("toxic duration {0:?} is too long")]
    DurationTooLong(Duration),
}

/// Which side of a proxied connection a toxic acts on.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToxicDirection {
    /// Data flowing from the upstream back to the client.
    Downstream,
    /// Data flowing from the client to the upstream.
    Upstream,
}

impl ToxicDirection {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Downstream => "downstream",
            Self::Upstream => "upstream",
        }
    }
}

/// A network fault Toxiproxy applies to a proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Toxic {
    /// Delays data by `latency`, plus or minus up to `jitter`.
    Latency {
        /// Added delay.
        latency: Duration,
        /// Random spread around the delay.
        jitter: Duration,
    },
    /// Resets the connection with a TCP RST after `after`, like a crashed peer.
    ResetPeer {
        /// Time before the reset; zero resets on the first data.
        after: Duration,
    },
    /// Stops all data and closes the connection after `after`; zero never closes it.
    Timeout {
        /// Time before the close.
        after: Duration,
    },
}

impl Toxic {
    const fn kind(self) -> &'static str {
        match self {
            Self::Latency { .. } => "latency",
            Self::ResetPeer { .. } => "reset_peer",
            Self::Timeout { .. } => "timeout",
        }
    }

    fn attributes(self) -> Result<Value, ToxiproxyError> {
        Ok(match self {
            Self::Latency { latency, jitter } => {
                json!({ "latency": millis(latency)?, "jitter": millis(jitter)? })
            }
            Self::ResetPeer { after } | Self::Timeout { after } => {
                json!({ "timeout": millis(after)? })
            }
        })
    }
}

fn millis(duration: Duration) -> Result<u64, ToxiproxyError> {
    u64::try_from(duration.as_millis()).map_err(|_| ToxiproxyError::DurationTooLong(duration))
}

/// A running Toxiproxy container and its API client.
///
/// Keep this value alive while proxies are in use. Dropping it invokes Testcontainers'
/// container cleanup behavior.
pub struct Toxiproxy {
    api_url: String,
    host: String,
    client: reqwest::Client,
    next_port: AtomicU16,
    container: ContainerAsync<GenericImage>,
}

impl Toxiproxy {
    /// Creates a proxy that forwards to `upstream`, an `ip:port` reachable from the
    /// Toxiproxy container.
    ///
    /// # Errors
    ///
    /// Returns [`ToxiproxyError::NoFreeProxyPort`] when every published port is taken, and
    /// [`ToxiproxyError::Http`], [`ToxiproxyError::Api`] or [`ToxiproxyError::Container`]
    /// when the proxy cannot be created or its host port cannot be read.
    pub async fn proxy(
        &self,
        name: &str,
        upstream: &str,
    ) -> Result<ToxiproxyProxy<'_>, ToxiproxyError> {
        let listen_port = self.claim_port()?;
        self.send(
            self.client
                .post(format!("{}/proxies", self.api_url))
                .json(&json!({
                    "name": name,
                    "listen": format!("0.0.0.0:{listen_port}"),
                    "upstream": upstream,
                    "enabled": true,
                })),
        )
        .await?;
        let host_port = self.container.get_host_port_ipv4(listen_port).await?;
        Ok(ToxiproxyProxy {
            toxiproxy: self,
            name: name.to_owned(),
            address: format!("{}:{host_port}", self.host),
        })
    }

    /// Creates a proxy to `container_port` of another container on the default bridge.
    ///
    /// # Errors
    ///
    /// Fails like [`Self::proxy`], or with [`ToxiproxyError::Container`] when the target's
    /// bridge address cannot be read.
    pub async fn proxy_container<I: Image>(
        &self,
        name: &str,
        target: &ContainerAsync<I>,
        container_port: u16,
    ) -> Result<ToxiproxyProxy<'_>, ToxiproxyError> {
        let ip = target.get_bridge_ip_address().await?;
        self.proxy(name, &format!("{ip}:{container_port}")).await
    }

    /// Enables every proxy and removes every toxic.
    ///
    /// # Errors
    ///
    /// Returns [`ToxiproxyError::Http`] or [`ToxiproxyError::Api`] when the reset fails.
    pub async fn reset(&self) -> Result<(), ToxiproxyError> {
        self.send(self.client.post(format!("{}/reset", self.api_url)))
            .await
    }

    /// Returns the host-accessible API base URL.
    #[must_use]
    pub fn api_url(&self) -> &str {
        &self.api_url
    }

    /// Returns the underlying Testcontainers guard for advanced test setup.
    #[must_use]
    pub const fn container(&self) -> &ContainerAsync<GenericImage> {
        &self.container
    }

    fn claim_port(&self) -> Result<u16, ToxiproxyError> {
        let offset = self.next_port.fetch_add(1, Ordering::Relaxed);
        if offset >= PROXY_PORT_COUNT {
            return Err(ToxiproxyError::NoFreeProxyPort);
        }
        Ok(FIRST_PROXY_PORT + offset)
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<(), ToxiproxyError> {
        let response = request.send().await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let body = response.text().await?;
        Err(ToxiproxyError::Api {
            status: status.as_u16(),
            body,
        })
    }
}

impl fmt::Debug for Toxiproxy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Toxiproxy")
            .field("api_url", &self.api_url)
            .field("container", &self.container)
            .finish_non_exhaustive()
    }
}

/// One proxy on a [`Toxiproxy`] fixture.
#[derive(Debug)]
pub struct ToxiproxyProxy<'a> {
    toxiproxy: &'a Toxiproxy,
    name: String,
    address: String,
}

impl ToxiproxyProxy<'_> {
    /// Returns the host-accessible `host:port` that clients connect to instead of the
    /// upstream.
    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    /// Returns the proxy name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Adds `toxic` to every connection in `direction` and returns the toxic's name.
    ///
    /// # Errors
    ///
    /// Returns [`ToxiproxyError::DurationTooLong`] for a duration past `u64` milliseconds,
    /// and [`ToxiproxyError::Http`] or [`ToxiproxyError::Api`] when the API rejects it.
    pub async fn add_toxic(
        &self,
        toxic: Toxic,
        direction: ToxicDirection,
    ) -> Result<String, ToxiproxyError> {
        let toxic_name = format!("{}_{}", toxic.kind(), direction.as_str());
        let body = json!({
            "name": toxic_name,
            "type": toxic.kind(),
            "stream": direction.as_str(),
            "toxicity": ALWAYS,
            "attributes": toxic.attributes()?,
        });
        self.toxiproxy
            .send(
                self.toxiproxy
                    .client
                    .post(format!("{}/toxics", self.url()))
                    .json(&body),
            )
            .await?;
        Ok(toxic_name)
    }

    /// Removes the toxic named `toxic_name`.
    ///
    /// # Errors
    ///
    /// Returns [`ToxiproxyError::Http`] or [`ToxiproxyError::Api`] when the toxic does not
    /// exist or the request fails.
    pub async fn remove_toxic(&self, toxic_name: &str) -> Result<(), ToxiproxyError> {
        self.toxiproxy
            .send(
                self.toxiproxy
                    .client
                    .delete(format!("{}/toxics/{toxic_name}", self.url())),
            )
            .await
    }

    /// Closes the listener and every open connection, or reopens it, like a link going
    /// down and coming back.
    ///
    /// # Errors
    ///
    /// Returns [`ToxiproxyError::Http`] or [`ToxiproxyError::Api`] when the update fails.
    pub async fn set_enabled(&self, enabled: bool) -> Result<(), ToxiproxyError> {
        self.toxiproxy
            .send(
                self.toxiproxy
                    .client
                    .patch(self.url())
                    .json(&json!({ "enabled": enabled })),
            )
            .await
    }

    fn url(&self) -> String {
        format!("{}/proxies/{}", self.toxiproxy.api_url, self.name)
    }
}

/// Starts a disposable Toxiproxy `2.13.0` container (pinned by digest).
///
/// The container publishes its API and a fixed set of proxy ports. Create proxies with
/// [`Toxiproxy::proxy`] or [`Toxiproxy::proxy_container`].
///
/// # Errors
///
/// Returns [`ToxiproxyError::Container`] when Docker cannot start the image or report its
/// address, and [`ToxiproxyError::Http`] when the API client cannot be built.
pub async fn start_toxiproxy() -> Result<Toxiproxy, ToxiproxyError> {
    let image = (0..PROXY_PORT_COUNT).fold(
        GenericImage::new(TOXIPROXY_IMAGE_NAME, TOXIPROXY_IMAGE_TAG)
            .with_exposed_port(TOXIPROXY_API_PORT.tcp())
            .with_wait_for(WaitFor::message_on_either_std(
                "Starting Toxiproxy HTTP server",
            )),
        |image, offset| image.with_exposed_port((FIRST_PROXY_PORT + offset).tcp()),
    );
    let request = image.with_labels(crate::container_labels());
    let container = request.start_container().await?;
    let host = container.get_host().await?.to_string();
    let api_port = container.get_host_port_ipv4(TOXIPROXY_API_PORT).await?;
    let client = crate::test_http_client().build()?;
    Ok(Toxiproxy {
        api_url: format!("http://{host}:{api_port}"),
        host,
        client,
        next_port: AtomicU16::new(0),
        container,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toxics_serialize_to_the_api_attributes() {
        let latency = Toxic::Latency {
            latency: Duration::from_millis(250),
            jitter: Duration::from_millis(10),
        };
        assert_eq!(latency.kind(), "latency");
        assert_eq!(
            latency.attributes().expect("attributes"),
            json!({ "latency": 250, "jitter": 10 })
        );
        let reset = Toxic::ResetPeer {
            after: Duration::from_millis(5),
        };
        assert_eq!(reset.kind(), "reset_peer");
        assert_eq!(
            reset.attributes().expect("attributes"),
            json!({ "timeout": 5 })
        );
        let timeout = Toxic::Timeout {
            after: Duration::ZERO,
        };
        assert_eq!(timeout.kind(), "timeout");
        assert_eq!(
            timeout.attributes().expect("attributes"),
            json!({ "timeout": 0 })
        );
    }

    #[test]
    fn oversized_durations_are_rejected() {
        let toxic = Toxic::Timeout {
            after: Duration::MAX,
        };
        assert!(matches!(
            toxic.attributes(),
            Err(ToxiproxyError::DurationTooLong(_))
        ));
    }
}
