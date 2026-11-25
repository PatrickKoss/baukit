//! Integer token-bucket step used by the in-memory store and mirrored by the Redis script.
//!
//! Token counts are scaled by the period in milliseconds: one millisecond refills
//! `rate` units (the quota's requests per period) and one request costs `period_ms`
//! units. Integer units keep the Rust kernel and the Lua script bit-for-bit equal.

use crate::Quota;

/// Largest scaled value the Lua script represents exactly in a double.
pub const MAX_SCALED: u64 = 1 << 53;

/// A quota expressed in scaled integer units.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScaledQuota {
    /// Units refilled per millisecond.
    pub rate: u64,
    /// Bucket capacity in units.
    pub capacity: u64,
    /// Units one request consumes.
    pub cost: u64,
}

impl ScaledQuota {
    /// Converts a quota, clamping every value to [`MAX_SCALED`].
    #[must_use]
    pub fn from_quota(quota: Quota) -> Self {
        let period_ms = u64::try_from(quota.period().as_millis())
            .unwrap_or(u64::MAX)
            .clamp(1, MAX_SCALED);
        Self {
            rate: quota.requests_per_period().min(MAX_SCALED),
            capacity: quota.capacity().saturating_mul(period_ms).min(MAX_SCALED),
            cost: period_ms,
        }
    }
}

/// Refills a bucket last updated at `last_ms` up to `now_ms`, capped at `capacity`.
#[must_use]
pub fn refill(tokens: u64, last_ms: u64, now_ms: u64, rate: u64, capacity: u64) -> u64 {
    let elapsed = now_ms.saturating_sub(last_ms);
    tokens
        .saturating_add(elapsed.saturating_mul(rate))
        .min(capacity)
}

/// Refills the bucket, then takes `cost` units when available.
///
/// Returns `(allowed, tokens_after, retry_ms)`. `retry_ms` is zero when allowed and
/// otherwise the milliseconds until `cost` units are available.
#[must_use]
pub fn admit(
    tokens: u64,
    last_ms: u64,
    now_ms: u64,
    rate: u64,
    capacity: u64,
    cost: u64,
) -> (bool, u64, u64) {
    let refilled = refill(tokens, last_ms, now_ms, rate, capacity);
    match refilled.checked_sub(cost) {
        Some(left) => (true, left, 0),
        None => (false, refilled, (cost - refilled).div_ceil(rate.max(1))),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn admits_until_empty_then_reports_retry() {
        let (allowed, tokens, retry) = admit(1_000, 0, 0, 1, 2_000, 1_000);
        assert!(allowed);
        assert_eq!((tokens, retry), (0, 0));
        assert_eq!(admit(0, 0, 400, 1, 2_000, 1_000), (false, 400, 600));
        assert_eq!(admit(0, 0, 1_000, 1, 2_000, 1_000), (true, 0, 0));
    }

    #[test]
    fn huge_periods_clamp_instead_of_overflowing() {
        let quota = Quota::new(u64::MAX, Duration::MAX, 0).expect("quota");
        let scaled = ScaledQuota::from_quota(quota);
        assert_eq!(scaled.capacity, MAX_SCALED);
        assert_eq!(scaled.cost, MAX_SCALED);
    }
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// K-11: one `admit` step never exceeds capacity or underflows, and admission
    /// implies `cost <= refilled tokens`. Inputs are unbounded `u64`; no loops, no unwind.
    #[kani::proof]
    fn admit_keeps_tokens_in_bounds() {
        let (tokens, last, now, rate, capacity, cost): (u64, u64, u64, u64, u64, u64) = (
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
            kani::any(),
        );
        kani::assume(tokens <= capacity);
        let refilled = refill(tokens, last, now, rate, capacity);
        let (allowed, after, retry) = admit(tokens, last, now, rate, capacity, cost);
        assert!(after <= capacity);
        assert!(refilled <= capacity);
        if allowed {
            assert!(cost <= refilled);
            assert!(after == refilled - cost);
            assert!(retry == 0);
        } else {
            assert!(cost > refilled);
            assert!(after == refilled);
        }
    }
}
