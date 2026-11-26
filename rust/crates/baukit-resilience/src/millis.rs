//! Whole-millisecond arithmetic shared by backoff and breaker, so every
//! language replays the same integers.

use std::time::Duration;

pub(crate) fn to_millis(duration: Duration) -> u32 {
    u32::try_from(duration.as_millis()).unwrap_or(u32::MAX)
}

pub(crate) fn from_millis(millis: u32) -> Duration {
    Duration::from_millis(u64::from(millis))
}

/// Returns `floor(millis * factor)` for `factor` in `[0, 1)`.
pub(crate) fn scale_floor(millis: u32, factor: f64) -> u32 {
    let scaled = (f64::from(millis) * factor).floor();
    let Ok(as_secs) = Duration::try_from_secs_f64(scaled) else {
        return 0;
    };
    u32::try_from(as_secs.as_secs())
        .unwrap_or(millis)
        .min(millis)
}

/// Returns `base * multiplier^exponent`, saturating at `u32::MAX`.
pub(crate) fn grow(base: u32, multiplier: u32, exponent: u32) -> u32 {
    multiplier
        .checked_pow(exponent)
        .map_or(u32::MAX, |factor| base.saturating_mul(factor))
}
