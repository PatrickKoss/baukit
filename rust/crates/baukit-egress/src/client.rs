use std::{
    error::Error as StdError,
    fmt,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use axum::http::{HeaderMap, Method, StatusCode};
use baukit_http::{RetryHeaderOptions, classify_http_status_with_options};
use reqwest::{
    dns::{Addrs, Name, Resolve, Resolving},
    redirect::Policy,
};
use tracing::{Instrument, field::Empty};
use url::{Host, Url};

use crate::{
    AddressPolicy, DestinationRejection, EgressError, EgressOptions, Resolver, SystemResolver,
    resolve_destination,
};

const TARGET: &str = "baukit_egress";

/// An outbound request for a [`GuardedClient`].
///
/// Its `Debug` output names only the method, host, header names, and body
/// length, so logging a request does not leak a URL secret or a header value.
#[derive(Clone)]
pub struct EgressRequest {
    method: Method,
    url: Url,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl EgressRequest {
    /// Creates a request with no headers and an empty body.
    #[must_use]
    pub fn new(method: Method, url: Url) -> Self {
        Self {
            method,
            url,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    /// Creates a `POST` request.
    #[must_use]
    pub fn post(url: Url) -> Self {
        Self::new(Method::POST, url)
    }

    /// Creates a `GET` request.
    #[must_use]
    pub fn get(url: Url) -> Self {
        Self::new(Method::GET, url)
    }

    /// Replaces the request headers.
    #[must_use]
    pub fn with_headers(mut self, headers: HeaderMap) -> Self {
        self.headers = headers;
        self
    }

    /// Replaces the request body.
    #[must_use]
    pub fn with_body(mut self, body: impl Into<Vec<u8>>) -> Self {
        self.body = body.into();
        self
    }

    /// Returns the request method.
    #[must_use]
    pub const fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the destination URL.
    #[must_use]
    pub const fn url(&self) -> &Url {
        &self.url
    }

    /// Returns the request headers.
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the request body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for EgressRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EgressRequest")
            .field("method", &self.method)
            .field("host", &self.url.host_str())
            .field("headers", &self.headers.keys().collect::<Vec<_>>())
            .field("body_len", &self.body.len())
            .finish()
    }
}

/// A `2xx` response whose body was read within the configured limit.
#[derive(Clone, Debug)]
pub struct EgressResponse {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl EgressResponse {
    /// Returns the response status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        self.status
    }

    /// Returns the response headers.
    #[must_use]
    pub const fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// Returns the response body.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// Takes the response body.
    #[must_use]
    pub fn into_body(self) -> Vec<u8> {
        self.body
    }
}

/// An HTTP client that reaches only destinations the address policy allows.
///
/// Every new connection resolves the host through the [`Resolver`] port,
/// rejects the lookup when any answer is disallowed, and connects only to the
/// answers it checked. Redirects are never followed, proxies from the
/// environment are ignored, and every request is bounded by the timeouts and
/// the response body limit in [`EgressOptions`]. Cloning is cheap and shares
/// the connection pool.
#[derive(Clone)]
pub struct GuardedClient {
    client: reqwest::Client,
    options: EgressOptions,
}

/// The underlying HTTP client could not be built.
#[derive(Debug, thiserror::Error)]
#[error("egress HTTP client could not be built")]
pub struct EgressClientError(#[source] reqwest::Error);

impl GuardedClient {
    /// Builds a client that resolves through the operating system.
    pub fn new(options: EgressOptions) -> Result<Self, EgressClientError> {
        Self::with_resolver(Arc::new(SystemResolver), options)
    }

    /// Builds a client that resolves through `resolver`.
    pub fn with_resolver(
        resolver: Arc<dyn Resolver>,
        options: EgressOptions,
    ) -> Result<Self, EgressClientError> {
        let pinned = PinnedResolver {
            resolver,
            policy: options.policy(),
            timeout: options.resolve_timeout(),
        };
        let client = reqwest::Client::builder()
            .dns_resolver(Arc::new(pinned))
            .redirect(Policy::none())
            .no_proxy()
            .referer(false)
            .https_only(!options.policy().allows_plain_http())
            .connect_timeout(options.connect_timeout())
            .timeout(options.request_timeout())
            .build()
            .map_err(EgressClientError)?;
        Ok(Self { client, options })
    }

    /// Returns the options this client was built with.
    #[must_use]
    pub const fn options(&self) -> &EgressOptions {
        &self.options
    }

    /// Sends `request` and reads the response body.
    ///
    /// A status outside `2xx` becomes [`EgressError::Status`], classified by
    /// [`baukit_http::classify_http_status`] without reading its body. A
    /// `Retry-After` delay above [`EgressOptions::max_retry_after`] is clamped.
    pub async fn execute(&self, request: EgressRequest) -> Result<EgressResponse, EgressError> {
        let span = tracing::info_span!(
            target: TARGET,
            "egress.request",
            http.request.method = %request.method,
            server.address = request.url.host_str().unwrap_or_default(),
            server.port = request.url.port_or_known_default(),
            http.response.status_code = Empty,
            error.type = Empty,
        );
        let outcome = self.send(request).instrument(span.clone()).await;
        match &outcome {
            Ok(response) => {
                span.record("http.response.status_code", response.status.as_u16());
            }
            Err(error) => {
                if let EgressError::Status { status, .. } = error {
                    span.record("http.response.status_code", status.as_u16());
                }
                span.record("error.type", error.code());
                tracing::debug!(target: TARGET, parent: &span, code = error.code(), %error, "egress request failed");
            }
        }
        outcome
    }

    async fn send(&self, request: EgressRequest) -> Result<EgressResponse, EgressError> {
        validate_destination(&request.url, self.options.policy())?;
        let response = self
            .client
            .request(request.method, request.url)
            .headers(request.headers)
            .body(request.body)
            .send()
            .await
            .map_err(send_error)?;
        let status = response.status();
        let headers = response.headers().clone();
        if !status.is_success() {
            let retry_headers =
                RetryHeaderOptions::default().with_max_retry_after(self.options.max_retry_after());
            let class = classify_http_status_with_options(status, &headers, retry_headers);
            return Err(EgressError::Status { status, class });
        }
        let body = read_body(response, self.options.max_response_bytes()).await?;
        Ok(EgressResponse {
            status,
            headers,
            body,
        })
    }
}

/// Checks a destination URL without resolving it.
///
/// The scheme must be `https`, or `http` when the policy allows plain HTTP.
/// The URL must have a host and no credentials or fragment. A host written as
/// an IP address must pass the policy, so an address literal never reaches a
/// connection. Host names are checked when they resolve.
pub fn validate_destination(url: &Url, policy: AddressPolicy) -> Result<(), EgressError> {
    let scheme_allowed =
        url.scheme() == "https" || (url.scheme() == "http" && policy.allows_plain_http());
    if !scheme_allowed {
        return Err(EgressError::Destination(DestinationRejection::Scheme));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(EgressError::Destination(DestinationRejection::Credentials));
    }
    if url.fragment().is_some() {
        return Err(EgressError::Destination(DestinationRejection::Fragment));
    }
    let literal = match url.host() {
        None => return Err(EgressError::Destination(DestinationRejection::MissingHost)),
        Some(Host::Ipv4(address)) => Some(IpAddr::V4(address)),
        Some(Host::Ipv6(address)) => Some(IpAddr::V6(address)),
        Some(Host::Domain(name)) => name.parse::<IpAddr>().ok(),
    };
    match literal {
        Some(address) if !policy.permits(address) => Err(EgressError::BlockedAddress),
        _ => Ok(()),
    }
}

struct PinnedResolver {
    resolver: Arc<dyn Resolver>,
    policy: AddressPolicy,
    timeout: Duration,
}

impl Resolve for PinnedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let resolver = Arc::clone(&self.resolver);
        let policy = self.policy;
        let timeout = self.timeout;
        let host = name.as_str().to_owned();
        Box::pin(async move {
            let answers = resolve_destination(resolver.as_ref(), &host, policy, timeout).await?;
            let addresses: Addrs = Box::new(
                answers
                    .into_iter()
                    .map(|address| SocketAddr::new(address, 0)),
            );
            Ok(addresses)
        })
    }
}

fn send_error(error: reqwest::Error) -> EgressError {
    if let Some(resolution) = resolution_error(&error) {
        return resolution;
    }
    if error.is_timeout() {
        return EgressError::Timeout;
    }
    EgressError::Transport(Box::new(error.without_url()))
}

fn resolution_error(error: &(dyn StdError + 'static)) -> Option<EgressError> {
    let mut current = Some(error);
    while let Some(error) = current {
        match error.downcast_ref::<EgressError>() {
            Some(EgressError::BlockedAddress) => return Some(EgressError::BlockedAddress),
            Some(EgressError::Timeout) => return Some(EgressError::Timeout),
            Some(_) => return Some(EgressError::Resolve),
            None => current = error.source(),
        }
    }
    None
}

async fn read_body(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>, EgressError> {
    let too_large = EgressError::ResponseTooLarge { limit };
    if response
        .content_length()
        .is_some_and(|length| length > u64::try_from(limit).unwrap_or(u64::MAX))
    {
        return Err(too_large);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(send_error)? {
        if body.len() + chunk.len() > limit {
            return Err(too_large);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}
