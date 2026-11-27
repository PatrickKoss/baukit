//! Startup retries for identity-provider discovery.

use std::{fmt, future::Future, time::Duration};

use baukit_resilience::startup::{RetryAttempt, StartupError};
#[cfg(test)]
use {baukit_resilience::startup::RETRY_MAX, tokio::time::Instant};

const DEPENDENCY: &str = "identity provider";

/// Why [`retry_within`] gave up.
#[derive(Debug)]
pub(crate) enum RetryFailure<E> {
    /// The error is not worth retrying, such as an issuer mismatch.
    Permanent(E),
    /// Every attempt inside the budget failed; holds the last error.
    Exhausted(E),
}

/// Runs `attempt` through [`baukit_resilience::startup::retry_within`] and
/// logs each retry.
pub(crate) async fn retry_within<T, E, F, Fut>(
    budget: Duration,
    is_retryable: impl Fn(&E) -> bool,
    attempt: F,
) -> Result<T, RetryFailure<E>>
where
    E: fmt::Display,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    baukit_resilience::startup::retry_within(DEPENDENCY, budget, is_retryable, log_retry, attempt)
        .await
        .map_err(|failure| match failure {
            StartupError::Permanent(error) => RetryFailure::Permanent(error),
            StartupError::BudgetExhausted { last, .. } => RetryFailure::Exhausted(last),
        })
}

fn log_retry<E: fmt::Display>(retry: &RetryAttempt<'_, E>) {
    tracing::warn!(
        retry = retry.attempt.saturating_sub(1),
        delay_ms = u64::try_from(retry.delay.as_millis()).unwrap_or(u64::MAX),
        error = %retry.error,
        "identity provider discovery failed, retrying"
    );
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;

    #[tokio::test(start_paused = true)]
    async fn retries_until_success_and_stops_on_permanent_errors() {
        let calls = AtomicU32::new(0);
        let result = retry_within(
            Duration::from_secs(60),
            |error: &&str| *error == "down",
            || {
                let call = calls.fetch_add(1, Ordering::SeqCst);
                async move { if call < 3 { Err("down") } else { Ok(call) } }
            },
        )
        .await;
        assert!(matches!(result, Ok(3)));

        let permanent = retry_within(
            Duration::from_secs(60),
            |_: &&str| false,
            || async { Err::<(), _>("mismatch") },
        )
        .await;
        assert!(matches!(
            permanent,
            Err(RetryFailure::Permanent("mismatch"))
        ));
    }

    #[tokio::test(start_paused = true)]
    async fn gives_up_with_the_last_error_after_the_budget() {
        let budget = Duration::from_secs(10);
        let started = Instant::now();
        let result = retry_within(budget, |_: &&str| true, || async { Err::<(), _>("down") }).await;
        assert!(matches!(result, Err(RetryFailure::Exhausted("down"))));
        assert!(started.elapsed() >= budget);
        assert!(started.elapsed() < budget + RETRY_MAX);
    }
}
