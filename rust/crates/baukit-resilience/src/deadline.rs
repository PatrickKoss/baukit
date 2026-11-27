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

#[cfg(feature = "task-deadline")]
pub use task::{current, scope};

#[cfg(feature = "task-deadline")]
mod task {
    use std::future::Future;

    use super::Deadline;

    tokio::task_local! {
        static CURRENT: Deadline;
    }

    /// Runs `future` with `deadline` as the [`current`] deadline.
    ///
    /// A nested scope keeps the earlier of the two deadlines. Tasks spawned
    /// inside the scope do not inherit it.
    pub async fn scope<F: Future>(deadline: Deadline, future: F) -> F::Output {
        let effective = current().map_or(deadline, |outer| outer.min(deadline));
        CURRENT.scope(effective, future).await
    }

    /// The deadline of the enclosing [`scope`], if any.
    #[must_use]
    pub fn current() -> Option<Deadline> {
        CURRENT.try_with(|deadline| *deadline).ok()
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

    #[tokio::test]
    async fn scopes_carry_the_earliest_deadline() {
        assert_eq!(current(), None);
        let start = Instant::now();
        let (Some(outer), Some(inner)) = (
            Deadline::after(start, Duration::from_secs(5)),
            Deadline::after(start, Duration::from_secs(2)),
        ) else {
            return;
        };
        let seen = scope(outer, async {
            let tighter = scope(inner, async { current() }).await;
            let looser = scope(outer, async { scope(outer, async { current() }).await }).await;
            (current(), tighter, looser)
        })
        .await;
        assert_eq!(seen, (Some(outer), Some(inner), Some(outer)));
        let narrowed = scope(inner, scope(outer, async { current() })).await;
        assert_eq!(narrowed, Some(inner));
    }
}
