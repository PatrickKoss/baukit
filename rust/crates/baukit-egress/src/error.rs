use std::fmt;

use axum::http::StatusCode;
use baukit_http::{RetryClass, classify_transport_error};

/// Why a destination URL was refused before any lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DestinationRejection {
    /// The scheme is not `https`, or not `http` under
    /// [`AddressPolicy::AllowLoopback`](crate::AddressPolicy::AllowLoopback).
    Scheme,
    /// The URL carries a user name or password.
    Credentials,
    /// The URL carries a fragment.
    Fragment,
    /// The URL has no host.
    MissingHost,
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
        })
    }
}

/// A guarded request that did not produce a successful response.
///
/// No variant carries the destination URL, so messages and logs built from
/// this error cannot leak a path, query, or credentials.
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
    /// Redirects land here too; the client never follows them.
    #[error("egress destination answered with status {status}")]
    Status {
        /// The status the destination returned.
        status: StatusCode,
        /// The status and retry headers classified by
        /// [`baukit_http::classify_http_status`].
        class: RetryClass,
    },
    /// The response body was larger than the configured limit.
    #[error("egress response body exceeded {limit} bytes")]
    ResponseTooLarge {
        /// The configured limit in bytes.
        limit: usize,
    },
}

impl EgressError {
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
