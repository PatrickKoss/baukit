use std::net::IpAddr;

use url::{Host, Url};

use crate::policy::is_endpoint_address;

/// An exact host and port allowed to reach a configured service.
///
/// Private and loopback addresses are accepted. Link-local, cloud metadata,
/// multicast and other reserved addresses stay blocked. HTTP needs a separate
/// opt-in. No wildcard, suffix or network matching is performed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowedEndpoint {
    host: String,
    port: u16,
    allow_http: bool,
}

/// An invalid endpoint allowance. The message never includes the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error(
    "endpoint allowance requires an exact host, a nonzero port and no metadata or reserved address"
)]
pub struct AllowedEndpointError;

impl AllowedEndpoint {
    /// Validates an exact DNS host or IP literal and port.
    /// IPv6 literals must be bracketed. URLs and wildcards are rejected.
    pub fn new(host: &str, port: u16, allow_http: bool) -> Result<Self, AllowedEndpointError> {
        if port == 0
            || host.is_empty()
            || host
                .chars()
                .any(|c| c.is_whitespace() || "*/?#@\\".contains(c))
        {
            return Err(AllowedEndpointError);
        }
        let parsed = Host::parse(host).map_err(|_| AllowedEndpointError)?;
        let address = match parsed {
            Host::Ipv4(address) => Some(IpAddr::V4(address)),
            Host::Ipv6(address) => Some(IpAddr::V6(address)),
            Host::Domain(ref name) => {
                if name.contains(':') || name.starts_with('.') || name.ends_with('.') {
                    return Err(AllowedEndpointError);
                }
                None
            }
        };
        if address.is_some_and(|address| !is_endpoint_address(address)) {
            return Err(AllowedEndpointError);
        }
        Ok(Self {
            host: parsed.to_string(),
            port,
            allow_http,
        })
    }

    /// Returns whether the URL has exactly this host and effective port.
    #[must_use]
    pub fn matches(&self, url: &Url) -> bool {
        url.host_str() == Some(self.host.as_str()) && url.port_or_known_default() == Some(self.port)
    }

    /// Returns the normalized host, with brackets around IPv6 literals.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns the exact port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.port
    }

    pub(crate) fn permits_host(&self, host: &str) -> bool {
        self.host
            .trim_matches(['[', ']'])
            .eq_ignore_ascii_case(host)
    }

    pub(crate) fn permits(&self, address: IpAddr) -> bool {
        is_endpoint_address(address)
    }

    pub(crate) const fn allows_http(&self) -> bool {
        self.allow_http
    }
}
