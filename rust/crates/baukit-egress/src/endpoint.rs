use std::net::IpAddr;

use url::{Host, Url};

use crate::policy::{is_internal_endpoint_address, is_metadata_address, is_public_address};

/// An exact scheme, host and port allowed to reach one service.
///
/// [`Self::new`] and [`Self::from_url`] build an operator-configured internal
/// endpoint, whose DNS answers may be private or loopback. [`Self::tenant`] and
/// [`Self::tenant_from_url`] build an endpoint from a tenant-supplied URL, which
/// accepts public answers only, so a tenant-controlled DNS name cannot point
/// inward. Link-local, cloud metadata, multicast and other reserved addresses
/// stay blocked for both. The HTTP flag selects HTTP only or HTTPS only. No
/// wildcard, suffix or network matching is performed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowedEndpoint {
    host: String,
    port: u16,
    allow_http: bool,
    origin: EndpointOrigin,
}

/// Who configured an [`AllowedEndpoint`], which decides its reachable addresses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EndpointOrigin {
    /// Set by the operator in deployment config; private and loopback answers pass.
    OperatorInternal,
    /// Supplied by a tenant or end user; only public answers pass.
    Tenant,
}

impl EndpointOrigin {
    fn permits(self, address: IpAddr) -> bool {
        match self {
            Self::OperatorInternal => is_internal_endpoint_address(address),
            Self::Tenant => !is_metadata_address(address) && is_public_address(address),
        }
    }
}

/// An invalid endpoint allowance. The message never includes the input.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error(
    "endpoint allowance requires an exact host, a nonzero port and no metadata or reserved address"
)]
pub struct AllowedEndpointError;

impl AllowedEndpoint {
    /// Validates an operator-configured internal host and port.
    ///
    /// Never pass a tenant-supplied host; use [`Self::tenant`] instead.
    /// IPv6 literals must be bracketed. URLs and wildcards are rejected.
    /// `allow_http` selects HTTP only when true and HTTPS only when false.
    ///
    /// # Errors
    ///
    /// Returns [`AllowedEndpointError`] for port zero, an empty host, a host with
    /// whitespace, a wildcard or URL characters, an unbracketed IPv6 literal, or a
    /// domain that does not parse or starts or ends with a dot.
    pub fn new(host: &str, port: u16, allow_http: bool) -> Result<Self, AllowedEndpointError> {
        Self::with_origin(host, port, allow_http, EndpointOrigin::OperatorInternal)
    }

    /// Validates a tenant-supplied host and port. Only public answers pass.
    ///
    /// # Errors
    ///
    /// Returns [`AllowedEndpointError`] in every case [`Self::new`] does, and for
    /// an IP literal that is not a public address.
    pub fn tenant(host: &str, port: u16, allow_http: bool) -> Result<Self, AllowedEndpointError> {
        Self::with_origin(host, port, allow_http, EndpointOrigin::Tenant)
    }

    fn with_origin(
        host: &str,
        port: u16,
        allow_http: bool,
        origin: EndpointOrigin,
    ) -> Result<Self, AllowedEndpointError> {
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
                if name.starts_with('.') || name.ends_with('.') {
                    return Err(AllowedEndpointError);
                }
                None
            }
        };
        if address.is_some_and(|address| !origin.permits(address)) {
            return Err(AllowedEndpointError);
        }
        Ok(Self {
            host: parsed.to_string(),
            port,
            allow_http,
            origin,
        })
    }

    /// Allows exactly the scheme, host and effective port of an operator-configured URL.
    ///
    /// `http` URLs select HTTP only and `https` URLs HTTPS only. Other schemes,
    /// URLs without a host and URLs with credentials are rejected.
    ///
    /// # Errors
    ///
    /// Returns [`AllowedEndpointError`] for a scheme other than `http` or `https`,
    /// a URL with credentials or without a host, or a host [`Self::new`] rejects.
    pub fn from_url(url: &Url) -> Result<Self, AllowedEndpointError> {
        Self::url_with_origin(url, EndpointOrigin::OperatorInternal)
    }

    /// Allows exactly the scheme, host and effective port of a tenant-supplied URL.
    ///
    /// # Errors
    ///
    /// Returns [`AllowedEndpointError`] in every case [`Self::from_url`] does, and
    /// for an IP literal host that is not a public address.
    pub fn tenant_from_url(url: &Url) -> Result<Self, AllowedEndpointError> {
        Self::url_with_origin(url, EndpointOrigin::Tenant)
    }

    fn url_with_origin(url: &Url, origin: EndpointOrigin) -> Result<Self, AllowedEndpointError> {
        let allow_http = match url.scheme() {
            "http" => true,
            "https" => false,
            _ => return Err(AllowedEndpointError),
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(AllowedEndpointError);
        }
        let host = url.host_str().ok_or(AllowedEndpointError)?;
        let port = url.port_or_known_default().ok_or(AllowedEndpointError)?;
        Self::with_origin(host, port, allow_http, origin)
    }

    /// Returns whether the URL has exactly this scheme, host and effective port.
    #[must_use]
    pub fn matches(&self, url: &Url) -> bool {
        let scheme = if self.allow_http { "http" } else { "https" };
        url.scheme() == scheme
            && url.host_str() == Some(self.host.as_str())
            && url.port_or_known_default() == Some(self.port)
    }

    /// Returns the normalized host, with brackets around IPv6 literals.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Returns who configured this endpoint.
    #[must_use]
    pub const fn origin(&self) -> EndpointOrigin {
        self.origin
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
        self.origin.permits(address)
    }

    pub(crate) const fn allows_http(&self) -> bool {
        self.allow_http
    }
}
