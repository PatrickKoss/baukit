use std::{num::NonZeroU32, path::PathBuf};

use baukit_resilience::SplitMixJitter;
use serde::Deserialize;

use super::*;
use crate::{
    FixedUtcInterval, RecurringJob,
    runner::attempt::{retry_backoff, retry_index},
};

const BUCKET: Duration = Duration::from_millis(100);
/// 1000 delays over a 32 s window put 3.1 in each 100 ms bucket on average.
/// For a binomial(1000, 1/320) bucket, 20 or more has probability below 1e-10,
/// while the old plain exponential put all 1000 in one bucket.
const MAX_PER_BUCKET: usize = 20;
const RECURRING_TYPE: &str = "recurring.job";
const DAY: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Deserialize)]
struct BackoffFile {
    cases: Vec<BackoffCase>,
}

#[derive(Deserialize)]
struct BackoffCase {
    strategy: String,
    initial_ms: u64,
    max_ms: u64,
    attempt: u32,
    fraction: f64,
    expected_delay_ms: u64,
}

#[test]
fn runner_retry_delay_matches_the_shared_backoff_vectors() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fixtures/resilience/backoff-v1.json");
    let file: BackoffFile =
        serde_json::from_str(&std::fs::read_to_string(path).expect("fixture")).expect("json");
    let cases: Vec<_> = file
        .cases
        .iter()
        .filter(|case| case.strategy == "full_jitter")
        .collect();
    assert!(!cases.is_empty());
    for case in cases {
        let config = WorkerConfig {
            retry_initial: Duration::from_millis(case.initial_ms),
            retry_max: Duration::from_millis(case.max_ms),
            ..WorkerConfig::default()
        };
        let attempts = case.attempt.saturating_add(1);
        assert_eq!(
            retry_delay(&config, attempts, case.fraction),
            Duration::from_millis(case.expected_delay_ms),
            "attempt {} fraction {}",
            case.attempt,
            case.fraction
        );
    }
}

#[test]
fn a_thousand_simultaneous_failures_spread_over_the_retry_window() {
    let config = WorkerConfig {
        retry_initial: Duration::from_secs(1),
        retry_max: Duration::from_secs(5 * 60),
        ..WorkerConfig::default()
    };
    let jitter = SharedJitter::new(SplitMixJitter::with_seed(42));
    let window = Duration::from_secs(32);
    let mut buckets = vec![0_usize; 320];
    for _ in 0..1000 {
        let delay = jitter.delay(&retry_backoff(&config), retry_index(6));
        assert!(delay < window);
        let index = usize::try_from(delay.as_millis() / BUCKET.as_millis()).expect("bucket");
        buckets[index] += 1;
    }
    let fullest = buckets.iter().copied().max().unwrap_or(0);
    assert!(fullest <= MAX_PER_BUCKET, "fullest bucket held {fullest}");
    assert!(buckets.iter().filter(|count| **count > 0).count() > 250);
}

fn idle_config() -> WorkerConfig {
    WorkerConfig {
        poll_interval: Duration::from_millis(250),
        idle_poll_max: Duration::from_secs(5),
        queue_sample_interval: Duration::from_secs(15),
        store_call_timeout: Duration::from_secs(5),
        ..test_config()
    }
}

fn store_queries(store: &FakeStore) -> usize {
    let claims = lock(&store.claimed_job_types).len();
    let samples = lock(&store.sampled_job_types).len();
    claims + 2 * samples
}

#[tokio::test(start_paused = true)]
async fn an_idle_runner_issues_less_than_one_query_per_second_after_thirty_seconds() {
    let store = Arc::new(FakeStore::with_jobs(0));
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(PendingHandler), idle_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::sleep(Duration::from_secs(30)).await;
    let before = store_queries(&store);
    tokio::time::sleep(Duration::from_secs(30)).await;
    let queries = store_queries(&store) - before;
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
    assert!(queries < 30, "{queries} queries in 30 s");
}

#[tokio::test(start_paused = true)]
async fn a_wakeup_claims_without_waiting_for_the_idle_delay() {
    let store = Arc::new(FakeStore::with_jobs(0));
    let handler = Arc::new(BlockingHandler::default());
    let wakeup = JobWakeup::new();
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), handler.clone(), idle_config())
        .expect("valid runner")
        .with_wakeup(wakeup.clone());
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::sleep(Duration::from_secs(30)).await;
    lock(&store.jobs).push_back(fake_job(1));
    wakeup.wake();
    tokio::time::timeout(Duration::from_millis(50), async {
        while handler.maximum.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("woken runner claims at once");
    shutdown.trigger();
    handler.release.notify_waiters();
    task.await.expect("runner task").expect("clean drain");
}

#[tokio::test]
async fn max_per_key_reaches_every_claim() {
    let store = Arc::new(FakeStore::with_jobs(0));
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let limit = NonZeroU32::new(2);
    let runner = WorkerRunner::new(
        store.clone(),
        Arc::new(PendingHandler),
        WorkerConfig {
            max_per_key: limit,
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::sleep(Duration::from_millis(30)).await;
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
    let seen = lock(&store.max_per_key_seen).clone();
    assert!(!seen.is_empty());
    assert!(seen.iter().all(|value| *value == limit));
}

fn recurring() -> RecurringJob {
    RecurringJob::new(
        RECURRING_TYPE,
        FixedUtcInterval::new(DAY).expect("interval"),
        3,
    )
}

struct FailingRecurringHandler;

impl JobHandler for FailingRecurringHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[RECURRING_TYPE]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Err(JobError::retryable("purge failed"))))
    }
}

#[tokio::test]
async fn a_terminally_failed_recurring_slot_enqueues_the_next_slot() {
    let interval = FixedUtcInterval::new(DAY).expect("interval");
    let slot = interval.slot_at(Timestamp::now()).expect("slot");
    let new = recurring().job_for(slot);
    let store = Arc::new(FakeStore::with_jobs(0));
    *lock(&store.failure_disposition) = FailureDisposition::Failed;
    lock(&store.jobs).push_back(Job {
        job_type: RECURRING_TYPE.to_owned(),
        payload: new.payload,
        attempts: 3,
        ..fake_job(0)
    });
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        store.clone(),
        Arc::new(FailingRecurringHandler),
        test_config(),
    )
    .expect("valid runner")
    .with_recurring(recurring())
    .expect("declared type");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(Duration::from_secs(1), async {
        while lock(&store.enqueued).is_empty() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("next slot enqueued");
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
    let next = interval.next_slot(slot, Timestamp::now()).expect("next");
    let enqueued = lock(&store.enqueued).clone();
    assert_eq!(enqueued.len(), 1);
    assert_eq!(enqueued[0].idempotency_key, Some(next.identifier()));
    assert_eq!(enqueued[0].run_after, next.starts_at());
}

#[tokio::test]
async fn enqueue_initial_skips_past_a_failed_current_slot() {
    let interval = FixedUtcInterval::new(DAY).expect("interval");
    let now = Timestamp::now();
    let current = interval.slot_at(now).expect("slot");
    let next = interval.next_slot(current, now).expect("next");
    let store = FakeStore::with_jobs(0);
    *lock(&store.enqueue_status) = JobStatus::Failed;
    recurring()
        .enqueue_initial(&store, now)
        .await
        .expect("enqueue");
    let keys: Vec<_> = lock(&store.enqueued)
        .iter()
        .map(|job| job.idempotency_key.clone())
        .collect();
    assert_eq!(keys, [Some(current.identifier()), Some(next.identifier())]);

    let store = FakeStore::with_jobs(0);
    recurring()
        .enqueue_initial(&store, now)
        .await
        .expect("enqueue");
    assert_eq!(lock(&store.enqueued).len(), 1, "a live slot is kept");
}

#[test]
fn recurring_registration_requires_a_declared_type() {
    let runner = WorkerRunner::new(
        Arc::new(FakeStore::with_jobs(0)),
        Arc::new(PendingHandler),
        test_config(),
    )
    .expect("valid runner");
    let error = runner
        .with_recurring(recurring())
        .err()
        .expect("undeclared type fails");
    assert!(error.to_string().contains("not declared"));
}
