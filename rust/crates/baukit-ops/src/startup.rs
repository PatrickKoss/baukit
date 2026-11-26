//! Bounded connect retries for process startup.
//!
//! A process that starts while a dependency is still coming up waits with
//! jittered backoff inside a startup budget instead of exiting on the first
//! failed connect.

use std::{error::Error, fmt, future::Future, time::Duration};

use baukit_resilience::{Backoff, JitterSource, SplitMixJitter};
use tokio::time::Instant;

const RETRY_INITIAL: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// Why [`retry_within`] gave up.
#[derive(Debug)]
pub enum StartupError<E> {
    /// The error will not go away by waiting, such as bad credentials or an invalid URL.
    Permanent(E),
    /// Every attempt inside the budget failed with a transient error.
    BudgetExhausted {
        /// Dependency name passed to [`retry_within`].
        dependency: &'static str,
        /// The budget that ran out.
        budget: Duration,
        /// Number of attempts made.
        attempts: u32,
        /// The error of the last attempt.
        last: E,
    },
}

impl<E> StartupError<E> {
    /// Returns the error of the last attempt.
    pub fn into_last(self) -> E {
        match self {
            Self::Permanent(error) | Self::BudgetExhausted { last: error, .. } => error,
        }
    }

    /// Returns `true` when the budget ran out on transient errors.
    #[must_use]
    pub const fn is_budget_exhausted(&self) -> bool {
        matches!(self, Self::BudgetExhausted { .. })
    }
}

impl<E: fmt::Display> fmt::Display for StartupError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Permanent(error) => write!(formatter, "startup connect failed: {error}"),
            Self::BudgetExhausted {
                dependency,
                budget,
                attempts,
                last,
            } => write!(
                formatter,
                "{dependency} still unavailable after {attempts} attempts in {}s: {last}",
                budget.as_secs()
            ),
        }
    }
}

impl<E: Error + 'static> Error for StartupError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Permanent(error) | Self::BudgetExhausted { last: error, .. } => Some(error),
        }
    }
}

/// Runs `attempt` until it succeeds, fails permanently, or `budget` runs out.
///
/// Errors for which `is_transient` returns `false` end the loop at once.
/// Each retry logs a warning with the dependency, attempt number and delay.
/// Delays use [`Backoff`] with full jitter between 250 ms and 5 s. An attempt
/// that starts before the budget ends runs to completion, so the total can
/// exceed `budget` by one connect timeout.
///
/// # Errors
///
/// [`StartupError::Permanent`] for a non-transient error and
/// [`StartupError::BudgetExhausted`] with the last error when the budget runs out.
pub async fn retry_within<T, E, F, Fut>(
    dependency: &'static str,
    budget: Duration,
    is_transient: impl Fn(&E) -> bool,
    mut attempt: F,
) -> Result<T, StartupError<E>>
where
    E: fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let backoff = Backoff::new(RETRY_INITIAL, RETRY_MAX);
    let mut jitter = SplitMixJitter::from_entropy();
    let deadline = Instant::now().checked_add(budget);
    let mut attempts = 0_u32;
    loop {
        attempts = attempts.saturating_add(1);
        let error = match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) if !is_transient(&error) => return Err(StartupError::Permanent(error)),
            Err(error) => error,
        };
        let remaining = deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if remaining.is_zero() {
            return Err(StartupError::BudgetExhausted {
                dependency,
                budget,
                attempts,
                last: error,
            });
        }
        let delay = backoff
            .next_delay(
                attempts.saturating_sub(1),
                &mut jitter as &mut dyn JitterSource,
            )
            .min(remaining);
        tracing::warn!(
            dependency,
            attempt = attempts,
            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            error = %error,
            "dependency unavailable at startup, retrying"
        );
        tokio::time::sleep(delay).await;
    }
}

#[cfg(test)]
mod tests;
