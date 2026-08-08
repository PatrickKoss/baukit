use std::{fmt, time::Duration};

use axum::http::HeaderValue;
use baukit_config::HttpConfig;

const DEFAULT_BODY_SIZE_LIMIT: usize = 2 * 1024 * 1024;
const DEFAULT_CONCURRENCY_LIMIT: usize = 1_024;
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Configuration for the shared HTTP layer stack.
///
/// CORS denies cross-origin access until explicit origins are supplied through
/// [`HttpOptions::with_allowed_origins`]. Wildcard origins are rejected, which
/// also prevents the invalid wildcard-with-credentials combination.
#[derive(Clone, Debug)]
pub struct HttpOptions {
    /// Maximum time allowed for one request, including concurrency queueing.
    pub request_timeout: Duration,
    /// Maximum accepted request body size in bytes.
    pub body_size_limit: usize,
    /// Maximum number of requests processed concurrently.
    pub concurrency_limit: usize,
    /// Whether browsers may send credentials to explicitly allowed origins.
    pub allow_credentials: bool,
    pub(crate) allowed_origins: Vec<HeaderValue>,
}

impl HttpOptions {
    /// Replaces the CORS origin allowlist.
    ///
    /// Each origin must be a valid HTTP header value and must not be `*`.
    pub fn with_allowed_origins<I, S>(mut self, origins: I) -> Result<Self, HttpOptionsError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.allowed_origins = parse_origins(origins)?;
        Ok(self)
    }

    /// Returns the explicit CORS origin allowlist.
    #[must_use]
    pub fn allowed_origins(&self) -> &[HeaderValue] {
        &self.allowed_origins
    }

    /// Builds options from Baukit's standard public-listener configuration.
    ///
    /// Listener address and port are intentionally ignored because they belong
    /// to runtime composition rather than an Axum layer.
    pub fn from_config(config: &HttpConfig) -> Result<Self, HttpOptionsError> {
        Self {
            request_timeout: config.request_timeout,
            body_size_limit: config.body_size_limit,
            concurrency_limit: config.concurrency_limit,
            allow_credentials: false,
            allowed_origins: parse_origins(&config.cors_allowed_origins)?,
        }
        .validate()
    }

    pub(crate) fn validate(self) -> Result<Self, HttpOptionsError> {
        if self.request_timeout.is_zero() {
            return Err(HttpOptionsError::ZeroRequestTimeout);
        }
        if self.body_size_limit == 0 {
            return Err(HttpOptionsError::ZeroBodySizeLimit);
        }
        if self.concurrency_limit == 0 {
            return Err(HttpOptionsError::ZeroConcurrencyLimit);
        }
        Ok(self)
    }
}

impl Default for HttpOptions {
    fn default() -> Self {
        Self {
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            body_size_limit: DEFAULT_BODY_SIZE_LIMIT,
            concurrency_limit: DEFAULT_CONCURRENCY_LIMIT,
            allow_credentials: false,
            allowed_origins: Vec::new(),
        }
    }
}

/// An invalid shared HTTP layer option.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HttpOptionsError {
    /// A CORS origin is not representable as an HTTP header value.
    InvalidOrigin(String),
    /// Wildcard CORS origins are deliberately unsupported.
    WildcardOrigin,
    /// Request timeout must be non-zero.
    ZeroRequestTimeout,
    /// Body size limit must be non-zero.
    ZeroBodySizeLimit,
    /// Concurrency limit must be non-zero.
    ZeroConcurrencyLimit,
}

impl fmt::Display for HttpOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidOrigin(origin) => write!(formatter, "invalid CORS origin `{origin}`"),
            Self::WildcardOrigin => formatter.write_str(
                "wildcard CORS origins are unsupported; provide explicit allowed origins",
            ),
            Self::ZeroRequestTimeout => formatter.write_str("request timeout must be non-zero"),
            Self::ZeroBodySizeLimit => formatter.write_str("body size limit must be non-zero"),
            Self::ZeroConcurrencyLimit => formatter.write_str("concurrency limit must be non-zero"),
        }
    }
}

impl std::error::Error for HttpOptionsError {}

fn parse_origins<I, S>(origins: I) -> Result<Vec<HeaderValue>, HttpOptionsError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    origins
        .into_iter()
        .map(|origin| {
            let origin = origin.as_ref();
            if origin == "*" {
                return Err(HttpOptionsError::WildcardOrigin);
            }
            HeaderValue::from_str(origin)
                .map_err(|_| HttpOptionsError::InvalidOrigin(origin.to_owned()))
        })
        .collect()
}
