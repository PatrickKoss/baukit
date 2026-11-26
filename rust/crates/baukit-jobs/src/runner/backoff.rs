use std::{
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use baukit_resilience::{Backoff, JitterSource, SplitMixJitter};

const STORE_RETRY_CAP: Duration = Duration::from_secs(30);

/// Whether the runner's last store call inside its loop succeeded.
///
/// Clones share one flag. Wire [`StoreHealth::is_healthy`] into readiness so a
/// worker reports not ready while its store fails.
#[derive(Clone, Debug)]
pub struct StoreHealth(Arc<AtomicBool>);

impl StoreHealth {
    pub(super) fn new() -> Self {
        Self(Arc::new(AtomicBool::new(true)))
    }

    /// Returns false from the first failing store call until the next success.
    #[must_use]
    pub fn is_healthy(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub(super) fn set(&self, healthy: bool) {
        self.0.store(healthy, Ordering::Release);
    }
}

/// The runner's jitter source, shared by the loop and every attempt.
#[derive(Clone)]
pub(super) struct SharedJitter(Arc<Mutex<Box<dyn JitterSource>>>);

impl SharedJitter {
    pub(super) fn new(source: impl JitterSource + 'static) -> Self {
        Self(Arc::new(Mutex::new(Box::new(source))))
    }

    pub(super) fn from_entropy() -> Self {
        Self::new(SplitMixJitter::from_entropy())
    }

    pub(super) fn delay(&self, backoff: &Backoff, attempt: u32) -> Duration {
        let mut source = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        backoff.next_delay(attempt, source.as_mut())
    }
}

/// Full-jitter backoff for store errors in the runner loop, capped at 30 s.
pub(super) struct StoreBackoff {
    backoff: Backoff,
    failures: u32,
    jitter: SharedJitter,
}

impl StoreBackoff {
    pub(super) fn new(base: Duration, jitter: SharedJitter) -> Self {
        Self {
            backoff: Backoff::new(base, STORE_RETRY_CAP),
            failures: 0,
            jitter,
        }
    }

    pub(super) const fn reset(&mut self) {
        self.failures = 0;
    }

    pub(super) fn next_delay(&mut self) -> Duration {
        let delay = self.jitter.delay(&self.backoff, self.failures);
        self.failures = self.failures.saturating_add(1);
        delay
    }
}

/// Empty-claim polling delay: doubles from the poll interval up to a cap and
/// resets after a claim or a wake-up.
pub(super) struct IdleBackoff {
    base: Duration,
    max: Duration,
    current: Duration,
}

impl IdleBackoff {
    pub(super) fn new(base: Duration, max: Duration) -> Self {
        let max = max.max(base);
        Self {
            base,
            max,
            current: base,
        }
    }

    pub(super) const fn reset(&mut self) {
        self.current = self.base;
    }

    pub(super) fn next_delay(&mut self) -> Duration {
        let delay = self.current;
        self.current = self.current.saturating_mul(2).min(self.max);
        delay
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedJitter(f64);

    impl JitterSource for FixedJitter {
        fn next_fraction(&mut self) -> f64 {
            self.0
        }
    }

    #[test]
    fn store_backoff_uses_full_jitter_and_caps_at_thirty_seconds() {
        let base = Duration::from_millis(100);
        let mut backoff = StoreBackoff::new(base, SharedJitter::new(FixedJitter(0.5)));
        assert_eq!(backoff.next_delay(), Duration::from_millis(50));
        assert_eq!(backoff.next_delay(), Duration::from_millis(100));
        assert_eq!(backoff.next_delay(), Duration::from_millis(200));
        for _ in 0..40 {
            assert!(backoff.next_delay() <= STORE_RETRY_CAP);
        }
        assert_eq!(backoff.next_delay(), STORE_RETRY_CAP / 2);
        backoff.reset();
        assert_eq!(backoff.next_delay(), Duration::from_millis(50));
    }

    #[test]
    fn idle_backoff_doubles_to_its_cap_and_resets() {
        let mut idle = IdleBackoff::new(Duration::from_millis(250), Duration::from_secs(5));
        let delays: Vec<_> = (0..7).map(|_| idle.next_delay().as_millis()).collect();
        assert_eq!(delays, [250, 500, 1000, 2000, 4000, 5000, 5000]);
        idle.reset();
        assert_eq!(idle.next_delay(), Duration::from_millis(250));
    }

    #[test]
    fn store_health_is_shared_between_clones() {
        let health = StoreHealth::new();
        let observer = health.clone();
        assert!(observer.is_healthy());
        health.set(false);
        assert!(!observer.is_healthy());
    }
}
