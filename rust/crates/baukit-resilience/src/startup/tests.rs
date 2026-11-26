use std::sync::atomic::{AtomicU32, Ordering};

use super::*;

const BUDGET: Duration = Duration::from_secs(10);

#[tokio::test(start_paused = true)]
async fn retries_transient_errors_until_success() {
    let calls = AtomicU32::new(0);
    let result = retry_within(
        "test",
        BUDGET,
        |error: &&str| *error == "down",
        |_| {},
        || {
            let call = calls.fetch_add(1, Ordering::SeqCst);
            async move { if call < 3 { Err("down") } else { Ok(call) } }
        },
    )
    .await;
    assert!(matches!(result, Ok(3)));
}

#[tokio::test(start_paused = true)]
async fn permanent_error_fails_without_waiting() {
    let started = Instant::now();
    let calls = AtomicU32::new(0);
    let result = retry_within(
        "test",
        BUDGET,
        |_: &&str| false,
        |_| {},
        || {
            calls.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>("bad password") }
        },
    )
    .await;
    assert!(matches!(
        result,
        Err(StartupError::Permanent("bad password"))
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(started.elapsed(), Duration::ZERO);
}

#[tokio::test(start_paused = true)]
async fn gives_up_with_the_last_error_after_the_budget() {
    let started = Instant::now();
    let mut delays = Vec::new();
    let result = retry_within(
        "postgres",
        BUDGET,
        |_: &&str| true,
        |retry: &RetryAttempt<'_, &str>| delays.push((retry.attempt, retry.delay)),
        || async { Err::<(), _>("down") },
    )
    .await;
    let Err(error) = result else {
        panic!("budget must run out");
    };
    assert!(error.is_budget_exhausted());
    let rendered = error.to_string();
    assert!(
        rendered.starts_with("postgres still unavailable after "),
        "{rendered}"
    );
    assert!(rendered.ends_with(" in 10s: down"), "{rendered}");
    let StartupError::BudgetExhausted { attempts, .. } = error else {
        panic!("budget exhausted");
    };
    assert!(attempts > 2, "{attempts}");
    assert_eq!(delays.len(), usize::try_from(attempts - 1).expect("small"));
    assert_eq!(delays.first().map(|(attempt, _)| *attempt), Some(1));
    let slept: Duration = delays.iter().map(|(_, delay)| *delay).sum();
    assert_eq!(slept, BUDGET);
    assert!(started.elapsed() >= BUDGET);
    assert!(started.elapsed() <= BUDGET);
}
