use std::{fmt, str::FromStr, sync::Arc, time::Duration};

use axum::http::HeaderName;
use reqwest::Certificate;

use crate::AddressPolicy;

const DEFAULT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
/// The longest header value kept by [`EgressOptions::with_error_headers`].
pub const MAX_ERROR_HEADER_VALUE_BYTES: usize = 4096;
const DEFAULT_MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

/// Limits and address policy for a [`GuardedClient`](crate::GuardedClient).
///
/// The defaults are [`AddressPolicy::PublicOnly`], a 3 second lookup timeout,
/// a 5 second connect timeout, a 10 second timeout for the whole request
/// including the body, a 1 MiB response body limit, and a 300 second cap on
/// the `Retry-After` delay reported in [`EgressError::Status`](crate::EgressError::Status).
/// Error bodies are not read unless [`Self::with_error_body_limit`] is set.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EgressOptions {
    policy: AddressPolicy,
    resolve_timeout: Duration,
    connect_timeout: Duration,
    request_timeout: Duration,
    max_response_bytes: usize,
    error_body_limit: Option<usize>,
    max_retry_after: Duration,
    forbidden_rate_limit: bool,
    redirect_responses: bool,
    retry_after_headers: Vec<HeaderName>,
    retry_after_first_field: bool,
    error_headers: Vec<HeaderName>,
    extra_roots: ExtraRoots,
}

/// A rejected [`EgressOptions`] value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum EgressOptionsError {
    /// A timeout was zero.
    #[error("egress {0} timeout must be greater than zero")]
    ZeroTimeout(&'static str),
    /// The response body limit was zero.
    #[error("egress response body limit must be greater than zero")]
    ZeroResponseLimit,
    /// The error body limit was zero.
    #[error("egress error body limit must be greater than zero")]
    ZeroErrorBodyLimit,
    /// The error body limit exceeded the response body limit.
    #[error("egress error body limit must not exceed the response body limit")]
    ErrorBodyLimitTooLarge,
    /// The `Retry-After` cap was zero.
    #[error("egress retry-after cap must be greater than zero")]
    ZeroRetryAfterCap,
    /// A retry-after header name was not a valid HTTP header name.
    #[error("egress retry-after header name is not a valid HTTP header name")]
    InvalidRetryHeaderName,
    /// An error header name was not a valid HTTP header name.
    #[error("egress error header name is not a valid HTTP header name")]
    InvalidErrorHeaderName,
    /// The root certificate PEM held no certificate or a malformed one.
    #[error("egress root certificate PEM holds no valid certificate")]
    InvalidRootCertificate,
}

/// Extra trusted roots, compared by the PEM they were parsed from.
#[derive(Clone, Default)]
struct ExtraRoots {
    pem: Vec<Arc<[u8]>>,
    certificates: Vec<Certificate>,
}

impl PartialEq for ExtraRoots {
    fn eq(&self, other: &Self) -> bool {
        self.pem == other.pem
    }
}

impl Eq for ExtraRoots {}

impl fmt::Debug for ExtraRoots {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExtraRoots")
            .field("certificates", &self.certificates.len())
            .finish()
    }
}

impl Default for EgressOptions {
    fn default() -> Self {
        Self {
            policy: AddressPolicy::PublicOnly,
            resolve_timeout: DEFAULT_RESOLVE_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            error_body_limit: None,
            max_retry_after: DEFAULT_MAX_RETRY_AFTER,
            forbidden_rate_limit: false,
            redirect_responses: false,
            retry_after_headers: Vec::new(),
            retry_after_first_field: false,
            error_headers: Vec::new(),
            extra_roots: ExtraRoots::default(),
        }
    }
}

impl EgressOptions {
    /// Returns redirect status and headers with an empty body, without following.
    ///
    /// The caller must send each next URL through the guard again.
    #[must_use]
    pub const fn with_redirect_responses(mut self) -> Self {
        self.redirect_responses = true;
        self
    }

    /// Returns whether redirects are returned with an empty body.
    #[must_use]
    pub const fn redirect_responses(&self) -> bool {
        self.redirect_responses
    }

    /// Sets which resolved addresses may be reached.
    #[must_use]
    pub fn with_policy(mut self, policy: AddressPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Sets the timeout for one DNS lookup.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroTimeout`] when `timeout` is zero.
    pub fn with_resolve_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.resolve_timeout = non_zero(timeout, "resolve")?;
        Ok(self)
    }

    /// Sets the timeout for establishing one TCP connection.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroTimeout`] when `timeout` is zero.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.connect_timeout = non_zero(timeout, "connect")?;
        Ok(self)
    }

    /// Sets the timeout for a whole request, from connect to the last body byte.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroTimeout`] when `timeout` is zero.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.request_timeout = non_zero(timeout, "request")?;
        Ok(self)
    }

    /// Sets the largest response body the client reads.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroResponseLimit`] when `limit` is zero and
    /// [`EgressOptionsError::ErrorBodyLimitTooLarge`] when it is below the configured
    /// error body limit.
    pub fn with_max_response_bytes(mut self, limit: usize) -> Result<Self, EgressOptionsError> {
        if limit == 0 {
            return Err(EgressOptionsError::ZeroResponseLimit);
        }
        if matches!(self.error_body_limit, Some(error_limit) if error_limit > limit) {
            return Err(EgressOptionsError::ErrorBodyLimitTooLarge);
        }
        self.max_response_bytes = limit;
        Ok(self)
    }

    /// Reads up to `limit` bytes of non-`2xx` bodies within the request timeout.
    ///
    /// Disabled by default. The limit must be nonzero and no larger than
    /// [`Self::max_response_bytes`]. Reaching it truncates the body without
    /// failing the request. Bodies may contain personal data; log only the
    /// [`ErrorBody::problem_code`](crate::ErrorBody::problem_code).
    /// If reading fails or times out, the status and retry class are preserved
    /// and the captured body is `None`.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroErrorBodyLimit`] when `limit` is zero and
    /// [`EgressOptionsError::ErrorBodyLimitTooLarge`] when it exceeds
    /// [`Self::max_response_bytes`].
    pub fn with_error_body_limit(mut self, limit: usize) -> Result<Self, EgressOptionsError> {
        if limit == 0 {
            return Err(EgressOptionsError::ZeroErrorBodyLimit);
        }
        if limit > self.max_response_bytes {
            return Err(EgressOptionsError::ErrorBodyLimitTooLarge);
        }
        self.error_body_limit = Some(limit);
        Ok(self)
    }

    /// Sets the largest `Retry-After` delay a status error reports.
    ///
    /// Longer delays from the upstream are clamped to `max`.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::ZeroRetryAfterCap`] when `max` is zero.
    pub fn with_max_retry_after(mut self, max: Duration) -> Result<Self, EgressOptionsError> {
        if max.is_zero() {
            return Err(EgressOptionsError::ZeroRetryAfterCap);
        }
        self.max_retry_after = max;
        Ok(self)
    }

    /// Recognizes rate limits on `403` responses using retry headers or
    /// `x-ratelimit-remaining: 0` with an optional `x-ratelimit-reset` epoch.
    ///
    /// Delays are clamped to [`Self::max_retry_after`]. Other `403` responses
    /// and all `401` responses stay [`RetryClass::Revoked`](baukit_http::RetryClass::Revoked).
    #[must_use]
    pub const fn with_forbidden_rate_limit(mut self) -> Self {
        self.forbidden_rate_limit = true;
        self
    }

    /// Reads retry delays from the named vendor headers, in order, before `Retry-After`.
    ///
    /// Use it for vendor rate-limit headers such as `fitbit-rate-limit-reset`
    /// (Fitbit), `x-ratelimit-reset` (Whoop) or `ratelimit-reset` (Polar) whose
    /// value is a delay in seconds. Each call appends to the list. Delays are
    /// clamped to [`Self::max_retry_after`], and headers that are not named are
    /// ignored. With [`Self::with_forbidden_rate_limit`] the named headers are
    /// also read on `403` and win over the `x-ratelimit-reset` epoch fallback,
    /// so do not name `x-ratelimit-reset` for an API that sends an epoch there.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::InvalidRetryHeaderName`] when a name is not
    /// a valid HTTP header name.
    pub fn with_retry_after_headers<I, S>(mut self, names: I) -> Result<Self, EgressOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            let name = HeaderName::from_str(name.as_ref())
                .map_err(|_| EgressOptionsError::InvalidRetryHeaderName)?;
            self.retry_after_headers.push(name);
        }
        Ok(self)
    }

    /// Keeps the named response headers on [`EgressError::Status`](crate::EgressError::Status).
    ///
    /// Disabled by default, so no header reaches the error unless listed. Only
    /// listed headers are copied, and a value longer than
    /// [`MAX_ERROR_HEADER_VALUE_BYTES`] is dropped. Use it for challenge
    /// headers such as `WWW-Authenticate` on a `401` from a registry.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::InvalidErrorHeaderName`] when a name is not
    /// a valid HTTP header name.
    pub fn with_error_headers<I, S>(mut self, names: I) -> Result<Self, EgressOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for name in names {
            let name = HeaderName::from_str(name.as_ref())
                .map_err(|_| EgressOptionsError::InvalidErrorHeaderName)?;
            self.error_headers.push(name);
        }
        Ok(self)
    }

    /// Parses only the first comma-separated field of vendor retry header values.
    ///
    /// For APIs such as Polar that send one delay per quota window, for example
    /// `ratelimit-reset: 30, 3600`. `Retry-After` is always parsed whole.
    #[must_use]
    pub const fn with_retry_after_first_field(mut self) -> Self {
        self.retry_after_first_field = true;
        self
    }

    /// Trusts the root certificates in `pem` in addition to the built-in roots.
    ///
    /// For upstreams such as FinTS bank endpoints that chain to a private or
    /// extra CA. The PEM may hold several certificates and is parsed here, once.
    /// Each call adds more roots. The built-in roots stay trusted; there is no
    /// option to replace them. The address policy still applies.
    ///
    /// # Errors
    ///
    /// Returns [`EgressOptionsError::InvalidRootCertificate`] when `pem` holds no
    /// certificate or a malformed one.
    pub fn with_extra_root_certificates_pem(
        mut self,
        pem: &[u8],
    ) -> Result<Self, EgressOptionsError> {
        let certificates = Certificate::from_pem_bundle(pem)
            .map_err(|_| EgressOptionsError::InvalidRootCertificate)?;
        if certificates.is_empty() {
            return Err(EgressOptionsError::InvalidRootCertificate);
        }
        self.extra_roots.pem.push(Arc::from(pem));
        self.extra_roots.certificates.extend(certificates);
        Ok(self)
    }

    /// Returns the vendor retry-after header names, in the order they are read.
    #[must_use]
    pub fn retry_after_headers(&self) -> &[HeaderName] {
        &self.retry_after_headers
    }

    /// Returns the header names kept on status errors.
    #[must_use]
    pub fn error_headers(&self) -> &[HeaderName] {
        &self.error_headers
    }

    /// Returns whether only the first field of vendor retry headers is parsed.
    #[must_use]
    pub const fn retry_after_first_field(&self) -> bool {
        self.retry_after_first_field
    }

    pub(crate) fn extra_root_certificates(&self) -> &[Certificate] {
        &self.extra_roots.certificates
    }

    /// Returns the address policy.
    #[must_use]
    pub const fn policy(&self) -> &AddressPolicy {
        &self.policy
    }

    /// Returns the DNS lookup timeout.
    #[must_use]
    pub const fn resolve_timeout(&self) -> Duration {
        self.resolve_timeout
    }

    /// Returns the TCP connect timeout.
    #[must_use]
    pub const fn connect_timeout(&self) -> Duration {
        self.connect_timeout
    }

    /// Returns the whole-request timeout.
    #[must_use]
    pub const fn request_timeout(&self) -> Duration {
        self.request_timeout
    }

    /// Returns the response body limit in bytes.
    #[must_use]
    pub const fn max_response_bytes(&self) -> usize {
        self.max_response_bytes
    }

    /// Returns the error body limit in bytes, or `None` when reading is disabled.
    #[must_use]
    pub const fn error_body_limit(&self) -> Option<usize> {
        self.error_body_limit
    }

    /// Returns the largest `Retry-After` delay a status error reports.
    #[must_use]
    pub const fn max_retry_after(&self) -> Duration {
        self.max_retry_after
    }

    /// Returns whether rate-limit headers on `403` responses are recognized.
    ///
    /// Disabled by default.
    #[must_use]
    pub const fn forbidden_rate_limit(&self) -> bool {
        self.forbidden_rate_limit
    }
}

const fn non_zero(timeout: Duration, name: &'static str) -> Result<Duration, EgressOptionsError> {
    if timeout.is_zero() {
        return Err(EgressOptionsError::ZeroTimeout(name));
    }
    Ok(timeout)
}
