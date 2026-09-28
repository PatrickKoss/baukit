use std::time::Duration;

use crate::AddressPolicy;

const DEFAULT_RESOLVE_TIMEOUT: Duration = Duration::from_secs(3);
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_MAX_RESPONSE_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_RETRY_AFTER: Duration = Duration::from_secs(300);

/// Limits and address policy for a [`GuardedClient`](crate::GuardedClient).
///
/// The defaults are [`AddressPolicy::PublicOnly`], a 3 second lookup timeout,
/// a 5 second connect timeout, a 10 second timeout for the whole request
/// including the body, a 1 MiB response body limit, and a 300 second cap on
/// the `Retry-After` delay reported in [`EgressError::Status`](crate::EgressError::Status).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EgressOptions {
    policy: AddressPolicy,
    resolve_timeout: Duration,
    connect_timeout: Duration,
    request_timeout: Duration,
    max_response_bytes: usize,
    max_retry_after: Duration,
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
    /// The `Retry-After` cap was zero.
    #[error("egress retry-after cap must be greater than zero")]
    ZeroRetryAfterCap,
}

impl Default for EgressOptions {
    fn default() -> Self {
        Self {
            policy: AddressPolicy::PublicOnly,
            resolve_timeout: DEFAULT_RESOLVE_TIMEOUT,
            connect_timeout: DEFAULT_CONNECT_TIMEOUT,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_retry_after: DEFAULT_MAX_RETRY_AFTER,
        }
    }
}

impl EgressOptions {
    /// Sets which resolved addresses may be reached.
    #[must_use]
    pub const fn with_policy(mut self, policy: AddressPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Sets the timeout for one DNS lookup.
    pub fn with_resolve_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.resolve_timeout = non_zero(timeout, "resolve")?;
        Ok(self)
    }

    /// Sets the timeout for establishing one TCP connection.
    pub fn with_connect_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.connect_timeout = non_zero(timeout, "connect")?;
        Ok(self)
    }

    /// Sets the timeout for a whole request, from connect to the last body byte.
    pub fn with_request_timeout(mut self, timeout: Duration) -> Result<Self, EgressOptionsError> {
        self.request_timeout = non_zero(timeout, "request")?;
        Ok(self)
    }

    /// Sets the largest response body the client reads.
    pub const fn with_max_response_bytes(
        mut self,
        limit: usize,
    ) -> Result<Self, EgressOptionsError> {
        if limit == 0 {
            return Err(EgressOptionsError::ZeroResponseLimit);
        }
        self.max_response_bytes = limit;
        Ok(self)
    }

    /// Sets the largest `Retry-After` delay a status error reports.
    ///
    /// Longer delays from the upstream are clamped to `max`.
    pub const fn with_max_retry_after(mut self, max: Duration) -> Result<Self, EgressOptionsError> {
        if max.is_zero() {
            return Err(EgressOptionsError::ZeroRetryAfterCap);
        }
        self.max_retry_after = max;
        Ok(self)
    }

    /// Returns the address policy.
    #[must_use]
    pub const fn policy(&self) -> AddressPolicy {
        self.policy
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

    /// Returns the largest `Retry-After` delay a status error reports.
    #[must_use]
    pub const fn max_retry_after(&self) -> Duration {
        self.max_retry_after
    }
}

const fn non_zero(timeout: Duration, name: &'static str) -> Result<Duration, EgressOptionsError> {
    if timeout.is_zero() {
        return Err(EgressOptionsError::ZeroTimeout(name));
    }
    Ok(timeout)
}
