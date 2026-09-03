use std::{
    error::Error,
    fmt,
    future::Future,
    pin::Pin,
    time::{Duration, SystemTime},
};

/// A token-bucket quota.
///
/// `requests_per_period` controls the refill rate. Capacity is that request
/// count plus `burst`, so burst represents additional immediately available
/// requests rather than replacing the steady-period allowance.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Quota {
    requests_per_period: u64,
    period: Duration,
    burst: u64,
    capacity: u64,
}

impl Quota {
    /// Creates and validates a quota.
    pub fn new(requests_per_period: u64, period: Duration, burst: u64) -> Result<Self, QuotaError> {
        if requests_per_period == 0 {
            return Err(QuotaError::ZeroRequests);
        }
        if period.is_zero() {
            return Err(QuotaError::ZeroPeriod);
        }
        let capacity = requests_per_period
            .checked_add(burst)
            .ok_or(QuotaError::CapacityOverflow)?;
        Ok(Self {
            requests_per_period,
            period,
            burst,
            capacity,
        })
    }

    /// Returns the number of tokens refilled per period.
    #[must_use]
    pub const fn requests_per_period(self) -> u64 {
        self.requests_per_period
    }

    /// Returns the refill period.
    #[must_use]
    pub const fn period(self) -> Duration {
        self.period
    }

    /// Returns the additional burst allowance.
    #[must_use]
    pub const fn burst(self) -> u64 {
        self.burst
    }

    /// Returns the bucket capacity (`requests_per_period + burst`).
    #[must_use]
    pub const fn capacity(self) -> u64 {
        self.capacity
    }

    pub(crate) fn idle_ttl(self) -> Duration {
        let nanos = self
            .period
            .as_nanos()
            .saturating_mul(u128::from(self.capacity))
            .div_ceil(u128::from(self.requests_per_period))
            .clamp(1_000_000, Duration::MAX.as_nanos());
        Duration::new(
            u64::try_from(nanos / 1_000_000_000).unwrap_or(u64::MAX),
            (nanos % 1_000_000_000) as u32,
        )
    }
}

/// Invalid token-bucket quota parameters.
#[derive(Clone, Copy, Debug, thiserror::Error, Eq, PartialEq)]
pub enum QuotaError {
    /// The refill count must be non-zero.
    #[error("requests_per_period must be non-zero")]
    ZeroRequests,
    /// The refill period must be non-zero.
    #[error("period must be non-zero")]
    ZeroPeriod,
    /// The steady count and burst cannot be represented as a `u64` capacity.
    #[error("requests_per_period plus burst overflows the bucket capacity")]
    CapacityOverflow,
}

/// Result of atomically checking and consuming one request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitDecision {
    /// Whether one token was consumed and the request may continue.
    pub allowed: bool,
    /// Whole tokens immediately remaining after this decision.
    pub remaining: u64,
    /// Delay until another request can be admitted; zero when allowed.
    pub retry_after: Duration,
}

impl RateLimitDecision {
    pub(crate) const fn allowed(remaining: u64) -> Self {
        Self {
            allowed: true,
            remaining,
            retry_after: Duration::ZERO,
        }
    }

    pub(crate) const fn limited(remaining: u64, retry_after: Duration) -> Self {
        Self {
            allowed: false,
            remaining,
            retry_after,
        }
    }
}

/// Failure to obtain a rate-limit decision from a store adapter.
pub struct RateLimitStoreError {
    source: Box<dyn Error + Send + Sync>,
}

impl RateLimitStoreError {
    /// Wraps an adapter-specific error.
    pub fn new(error: impl Error + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(error),
        }
    }

    /// Creates an unavailable-store error, primarily for custom adapters and tests.
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(std::io::Error::other(message.into()))
    }
}

impl fmt::Debug for RateLimitStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RateLimitStoreError")
            .field(&self.source)
            .finish()
    }
}

impl fmt::Display for RateLimitStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "rate-limit store failed: {}", self.source)
    }
}

impl Error for RateLimitStoreError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

/// Asynchronous persistence port for atomic token-bucket decisions.
pub trait RateLimitStore: Send + Sync {
    /// Atomically refills a key's bucket, then consumes one token when available.
    fn check_and_consume<'a>(
        &'a self,
        key: &'a str,
        quota: Quota,
    ) -> Pin<Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>>;
}

/// Store-level result of an atomic amount-budget decision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AmountBudgetStoreDecision {
    /// Whether the store added the requested amount.
    pub allowed: bool,
    /// Units left after the decision.
    pub remaining: u64,
}

/// Persistence port for atomic fixed-window amount-budget decisions.
pub trait AmountBudgetStore: Send + Sync {
    /// Adds `amount` when doing so would not exceed `limit`.
    ///
    /// The store must create or update the counter and set its expiry to
    /// `reset_at` in one atomic operation. `now` lets process-local adapters
    /// expire entries against an injected clock.
    fn check_and_consume_amount<'a>(
        &'a self,
        key: &'a str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>> + Send + 'a,
        >,
    >;

    /// Subtracts `amount` from the current consumed total, floored at zero.
    ///
    /// The store must update an existing counter and preserve its expiry at
    /// `reset_at` in one atomic operation. A missing key remains absent.
    fn release_amount<'a>(
        &'a self,
        key: &'a str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> Pin<
        Box<
            dyn Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>> + Send + 'a,
        >,
    >;
}
