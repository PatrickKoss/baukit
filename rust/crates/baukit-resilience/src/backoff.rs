//! Jittered exponential backoff.

use std::time::Duration;

use crate::jitter::{JitterSource, sanitize};
use crate::millis::{from_millis, grow, scale_floor, to_millis};

const FULL_JITTER_GROWTH: u32 = 2;
const DECORRELATED_GROWTH: u32 = 3;

/// How [`Backoff::delay`] spreads retries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackoffStrategy {
    /// `floor(min(max, initial * 2^attempt) * fraction)`.
    #[default]
    FullJitter,
    /// `initial + floor((min(max, initial * 3^attempt) - initial) * fraction)`.
    ///
    /// A stateless form of decorrelated jitter: the upper bound grows by the
    /// factor of three the classic variant applies to the previous delay, and
    /// the delay never drops below `initial`.
    DecorrelatedJitter,
}

/// Bounded, jittered retry delays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Backoff {
    /// Delay bound for attempt `0`.
    pub initial: Duration,
    /// Upper bound for every delay.
    pub max: Duration,
    /// Jitter strategy.
    pub strategy: BackoffStrategy,
}

impl Backoff {
    /// Full jitter between `initial` and `max`.
    #[must_use]
    pub const fn new(initial: Duration, max: Duration) -> Self {
        Self {
            initial,
            max,
            strategy: BackoffStrategy::FullJitter,
        }
    }

    /// Replaces the strategy.
    #[must_use]
    pub const fn with_strategy(mut self, strategy: BackoffStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    /// Returns the delay before retry `attempt` (zero-based) for a jitter
    /// `fraction` in `[0, 1)`. Pure; durations are whole milliseconds.
    #[must_use]
    pub fn delay(&self, attempt: u32, fraction: f64) -> Duration {
        let fraction = sanitize(fraction);
        let max = to_millis(self.max);
        let initial = to_millis(self.initial).min(max);
        let millis = match self.strategy {
            BackoffStrategy::FullJitter => {
                let cap = grow(initial, FULL_JITTER_GROWTH, attempt).min(max);
                scale_floor(cap, fraction)
            }
            BackoffStrategy::DecorrelatedJitter => {
                let upper = grow(initial, DECORRELATED_GROWTH, attempt).min(max);
                initial.saturating_add(scale_floor(upper.saturating_sub(initial), fraction))
            }
        };
        from_millis(millis)
    }

    /// Returns the delay before retry `attempt`, drawing the fraction from `jitter`.
    pub fn next_delay(&self, attempt: u32, jitter: &mut dyn JitterSource) -> Duration {
        self.delay(attempt, jitter.next_fraction())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECOND: Duration = Duration::from_secs(1);
    const MINUTE: Duration = Duration::from_secs(60);

    #[test]
    fn full_jitter_scales_the_exponential_cap() {
        let backoff = Backoff::new(SECOND, MINUTE);
        assert_eq!(backoff.delay(0, 0.5), Duration::from_millis(500));
        assert_eq!(backoff.delay(3, 0.5), Duration::from_secs(4));
        assert_eq!(backoff.delay(40, 0.5), Duration::from_secs(30));
        assert_eq!(backoff.delay(3, 0.0), Duration::ZERO);
    }

    #[test]
    fn decorrelated_jitter_never_drops_below_initial() {
        let backoff =
            Backoff::new(SECOND, MINUTE).with_strategy(BackoffStrategy::DecorrelatedJitter);
        assert_eq!(backoff.delay(0, 0.9), SECOND);
        assert_eq!(backoff.delay(2, 0.0), SECOND);
        assert_eq!(backoff.delay(2, 0.5), Duration::from_secs(5));
        assert!(backoff.delay(u32::MAX, 0.999) < MINUTE);
    }

    #[test]
    fn delays_never_exceed_max() {
        let backoff = Backoff::new(Duration::from_secs(10), SECOND);
        for attempt in 0..64 {
            assert!(backoff.delay(attempt, 0.999_999) < SECOND);
        }
    }
}
