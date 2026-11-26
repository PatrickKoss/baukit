//! A fixed point in time that bounds a chain of calls.

use std::time::{Duration, Instant};

/// The instant by which work must finish.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Deadline {
    at: Instant,
}

impl Deadline {
    /// A deadline at `at`.
    #[must_use]
    pub const fn at(at: Instant) -> Self {
        Self { at }
    }

    /// A deadline `budget` after `now`, or `None` if that overflows.
    #[must_use]
    pub fn after(now: Instant, budget: Duration) -> Option<Self> {
        now.checked_add(budget).map(Self::at)
    }

    /// The deadline instant.
    #[must_use]
    pub const fn instant(&self) -> Instant {
        self.at
    }

    /// Time left at `now`, zero once passed.
    #[must_use]
    pub fn remaining(&self, now: Instant) -> Duration {
        self.at.saturating_duration_since(now)
    }

    /// Whether the deadline has passed at `now`.
    #[must_use]
    pub fn is_expired(&self, now: Instant) -> bool {
        now >= self.at
    }

    /// `timeout`, shortened to the time left at `now`.
    #[must_use]
    pub fn cap(&self, timeout: Duration, now: Instant) -> Duration {
        timeout.min(self.remaining(now))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn caps_timeouts_to_the_remaining_budget() {
        let start = Instant::now();
        let Some(deadline) = Deadline::after(start, Duration::from_secs(5)) else {
            return;
        };
        let later = start + Duration::from_secs(3);
        assert_eq!(deadline.remaining(later), Duration::from_secs(2));
        assert_eq!(
            deadline.cap(Duration::from_secs(10), later),
            Duration::from_secs(2)
        );
        assert_eq!(
            deadline.cap(Duration::from_secs(1), later),
            Duration::from_secs(1)
        );
        assert!(!deadline.is_expired(later));
        let past = start + Duration::from_secs(9);
        assert_eq!(deadline.remaining(past), Duration::ZERO);
        assert!(deadline.is_expired(past));
    }
}
