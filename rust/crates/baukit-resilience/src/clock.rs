//! The clock port.

use std::time::Instant;

/// Source of the current monotonic time.
pub trait Clock: Send + Sync {
    /// Returns the current instant.
    fn now(&self) -> Instant;
}

/// [`Clock`] backed by [`Instant::now`].
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(feature = "test-support")]
pub use manual::ManualClock;

#[cfg(feature = "test-support")]
mod manual {
    use std::sync::{Mutex, PoisonError};
    use std::time::{Duration, Instant};

    use super::Clock;

    /// [`Clock`] that only moves when a test advances it.
    #[derive(Debug)]
    pub struct ManualClock {
        now: Mutex<Instant>,
    }

    impl ManualClock {
        /// Starts the clock at the current instant.
        #[must_use]
        pub fn new() -> Self {
            Self::starting_at(Instant::now())
        }

        /// Starts the clock at `start`.
        #[must_use]
        pub fn starting_at(start: Instant) -> Self {
            Self {
                now: Mutex::new(start),
            }
        }

        /// Moves the clock forward by `by`. Saturates instead of overflowing.
        pub fn advance(&self, by: Duration) {
            let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(next) = now.checked_add(by) {
                *now = next;
            }
        }
    }

    impl Default for ManualClock {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.now.lock().unwrap_or_else(PoisonError::into_inner)
        }
    }
}
