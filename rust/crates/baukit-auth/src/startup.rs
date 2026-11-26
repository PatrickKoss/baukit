//! Startup retries for identity-provider discovery.

use std::{future::Future, time::Duration};

use baukit_resilience::{Backoff, JitterSource, SplitMixJitter};
use tokio::time::Instant;

const RETRY_INITIAL: Duration = Duration::from_millis(250);
const RETRY_MAX: Duration = Duration::from_secs(5);

/// Why [`retry_within`] gave up.
#[derive(Debug)]
pub(crate) enum RetryFailure<E> {
    /// The error is not worth retrying, such as an issuer mismatch.
    Permanent(E),
    /// Every attempt inside the budget failed; holds the last error.
    Exhausted(E),
}

/// Runs `attempt` until it succeeds, fails permanently, or `budget` runs out.
///
/// Delays use [`Backoff`] with full jitter. An attempt that starts before the
/// budget ends runs to completion, so the total can exceed `budget` by one
/// request timeout.
pub(crate) async fn retry_within<T, E, F, Fut>(
    budget: Duration,
    is_retryable: impl Fn(&E) -> bool,
    mut attempt: F,
) -> Result<T, RetryFailure<E>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let backoff = Backoff::new(RETRY_INITIAL, RETRY_MAX);
    let mut jitter = SplitMixJitter::from_entropy();
    let deadline = Instant::now().checked_add(budget);
    let mut retry = 0_u32;
    loop {
        let error = match attempt().await {
            Ok(value) => return Ok(value),
            Err(error) if !is_retryable(&error) => return Err(RetryFailure::Permanent(error)),
            Err(error) => error,
        };
        let remaining = deadline.map_or(Duration::MAX, |deadline| {
            deadline.saturating_duration_since(Instant::now())
        });
        if remaining.is_zero() {
            return Err(RetryFailure::Exhausted(error));
        }
        let delay = backoff.next_delay(retry, &mut jitter as &mut dyn JitterSource);
        tracing::warn!(
            retry,
            delay_ms = u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
            "identity provider discovery failed, retrying"
        );
        tokio::time::sleep(delay.min(remaining)).await;
        retry = retry.saturating_add(1);
    }
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
