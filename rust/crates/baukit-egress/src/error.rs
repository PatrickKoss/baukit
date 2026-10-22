use std::fmt;

use axum::http::StatusCode;
use baukit_http::{RetryClass, classify_transport_error};

/// A bounded non-`2xx` response body, read only when explicitly enabled.
///
/// The bytes may contain personal data. `Debug` shows only their length,
/// the truncation flag, and the problem code from Baukit's error envelope.
#[derive(Clone, Eq, PartialEq)]
pub struct ErrorBody {
    bytes: Vec<u8>,
    truncated: bool,
    content_type: Option<String>,
    problem_code: Option<String>,
}

impl ErrorBody {
    pub(crate) fn new(bytes: Vec<u8>, truncated: bool, content_type: Option<String>) -> Self {
        let problem_code = serde_json::from_slice::<baukit_http::ErrorEnvelope>(&bytes)
            .ok()
            .map(|envelope| envelope.error.code);
        Self {
            bytes,
            truncated,
            content_type,
            problem_code,
        }
    }

    /// Returns the captured bytes, which may contain personal data.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns whether reading stopped at the limit without confirming the end.
    ///
    /// A body with an unknown length that ends exactly at the limit is marked
    /// truncated, since the client stops without waiting for more bytes.
    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    /// Returns the response's `Content-Type`, if present and valid as text.
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.content_type.as_deref()
    }

    /// Returns the code if the captured bytes parse as Baukit's error envelope.
    ///
    /// Non-JSON, incomplete JSON, and other JSON shapes return `None`.
    #[must_use]
    pub fn problem_code(&self) -> Option<&str> {
        self.problem_code.as_deref()
    }
}

impl fmt::Debug for ErrorBody {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ErrorBody")
            .field("len", &self.bytes.len())
            .field("truncated", &self.truncated)
            .field("problem_code", &self.problem_code())
            .finish()
    }
}

/// Why a destination URL was refused before any lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum DestinationRejection {
    /// The scheme is not `https`, or is `http` without the policy
    /// allowing plain HTTP.
    Scheme,
    /// The URL carries a user name or password.
    Credentials,
    /// The URL carries a fragment.
    Fragment,
    /// The URL has no host.
    MissingHost,
    /// The URL does not match the client's exact endpoint allowance.
    Endpoint,
}

impl DestinationRejection {
    /// Returns a stable snake_case reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Scheme => "scheme",
            Self::Credentials => "credentials",
            Self::Fragment => "fragment",
            Self::MissingHost => "missing_host",
            Self::Endpoint => "endpoint",
        }
    }
}

impl fmt::Display for DestinationRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Scheme => "the scheme is not allowed",
            Self::Credentials => "the URL carries credentials",
            Self::Fragment => "the URL carries a fragment",
            Self::MissingHost => "the URL has no host",
            Self::Endpoint => "the URL does not match the allowed endpoint",
        })
    }
}

/// A guarded request that did not produce a successful response.
///
/// No variant carries the destination URL, so messages and logs built from
/// this error cannot leak a path, query, or credentials.
/// Opted-in error bodies may contain personal data. `Display` omits them,
/// and `Debug` uses [`ErrorBody`]'s redacted representation.
#[derive(Debug, thiserror::Error)]
pub enum EgressError {
    /// The URL was refused before any lookup.
    #[error("egress destination is not allowed: {0}")]
    Destination(DestinationRejection),
    /// The host is, or resolved to, an address the policy does not allow.
    #[error("egress destination resolved to a disallowed address")]
    BlockedAddress,
    /// The lookup failed or returned no addresses.
    #[error("egress destination could not be resolved")]
    Resolve,
    /// The lookup, connection, or whole request ran past its timeout.
    #[error("egress request timed out")]
    Timeout,
    /// The connection failed or broke before a complete response arrived.
    #[error("egress request failed before a complete response arrived")]
    Transport(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The destination answered with a status outside `2xx`.
    ///
    /// Redirects land here unless redirect responses are enabled. The client
    /// never follows them.
    #[error("egress destination answered with status {status}")]
    Status {
        /// The status the destination returned.
        status: StatusCode,
        /// The status and retry headers classified by
        /// [`baukit_http::classify_http_status_with_options`], with the retry
        /// delay capped at [`EgressOptions::max_retry_after`](crate::EgressOptions::max_retry_after).
        /// `403` stays revoked unless
        /// [`EgressOptions::with_forbidden_rate_limit`](crate::EgressOptions::with_forbidden_rate_limit)
        /// is enabled and recognizes a rate limit. `401` always stays revoked.
        class: RetryClass,
        /// The captured body, if [`EgressOptions::with_error_body_limit`](crate::EgressOptions::with_error_body_limit)
        /// is enabled. Truncation at that limit is not an error.
        body: Option<ErrorBody>,
    },
    /// The response body was larger than the configured limit.
    #[error("egress response body exceeded {limit} bytes")]
    ResponseTooLarge {
        /// The configured limit in bytes.
        limit: usize,
    },
}

impl EgressError {
    /// Returns the captured status body, or `None` for other failures or when disabled.
    /// If reading fails or times out, the status and retry class are preserved
    /// and this returns `None`.
    #[must_use]
    pub const fn error_body(&self) -> Option<&ErrorBody> {
        match self {
            Self::Status { body, .. } => body.as_ref(),
            _ => None,
        }
    }

    /// Returns how a caller should react to this failure.
    ///
    /// Refused destinations and oversized responses are
    /// [`RetryClass::Permanent`]. Lookup and transport failures are
    /// [`RetryClass::Unavailable`], timeouts are [`RetryClass::Timeout`], and a
    /// status keeps its classified class.
    #[must_use]
    pub const fn retry_class(&self) -> RetryClass {
        match self {
            Self::Destination(_) | Self::BlockedAddress | Self::ResponseTooLarge { .. } => {
                RetryClass::Permanent
            }
            Self::Resolve | Self::Transport(_) => classify_transport_error(false),
            Self::Timeout => classify_transport_error(true),
            Self::Status { class, .. } => *class,
        }
    }

    /// Returns whether retrying the same request can succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.retry_class().is_retryable()
    }

    /// Returns a stable snake_case code that is safe to store and log.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Destination(_) => "destination_not_allowed",
            Self::BlockedAddress => "blocked_address",
            Self::Resolve => "resolve_failed",
            Self::Timeout => "timeout",
            Self::Transport(_) => "transport_failed",
            Self::Status { .. } => "upstream_status",
            Self::ResponseTooLarge { .. } => "response_too_large",
        }
    }
}
