use std::{
    collections::hash_map::RandomState,
    hash::{BuildHasher as _, Hasher as _},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const STORE_RETRY_CAP: Duration = Duration::from_secs(30);
const MAX_DOUBLINGS: u32 = 16;

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

pub(super) struct StoreBackoff {
    base: Duration,
    failures: u32,
}

impl StoreBackoff {
    pub(super) const fn new(base: Duration) -> Self {
        Self { base, failures: 0 }
    }

    pub(super) const fn reset(&mut self) {
        self.failures = 0;
    }

    pub(super) fn next_delay(&mut self) -> Duration {
        let exponent = self.failures.min(MAX_DOUBLINGS);
        self.failures = self.failures.saturating_add(1);
        let capped = self
            .base
            .saturating_mul(2_u32.saturating_pow(exponent))
            .min(STORE_RETRY_CAP);
        equal_jitter(capped)
    }
}

/// Keeps half of `delay` and randomizes the other half. Swap for
/// `baukit_resilience::Backoff` once that crate lands.
fn equal_jitter(delay: Duration) -> Duration {
    let half = delay / 2;
    let random = RandomState::new().build_hasher().finish();
    let fraction = f64::from(u32::try_from(random >> 32).unwrap_or(u32::MAX)) / f64::from(u32::MAX);
    half + half.mul_f64(fraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_backoff_doubles_with_jitter_and_caps_at_thirty_seconds() {
        let base = Duration::from_millis(100);
        let mut backoff = StoreBackoff::new(base);
        for failure in 0..4_u32 {
            let ceiling = base * 2_u32.pow(failure);
            let delay = backoff.next_delay();
            assert!(
                delay >= ceiling / 2 && delay <= ceiling,
                "{delay:?} for {ceiling:?}"
            );
        }
        for _ in 0..40 {
            assert!(backoff.next_delay() <= STORE_RETRY_CAP);
        }
        assert!(backoff.next_delay() >= STORE_RETRY_CAP / 2);
        backoff.reset();
        assert!(backoff.next_delay() <= base);
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
