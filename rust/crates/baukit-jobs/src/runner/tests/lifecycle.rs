use metrics_util::debugging::{DebugValue, DebuggingRecorder, Snapshotter};

use super::*;

const WAIT: Duration = Duration::from_secs(5);

struct SucceedingHandler;

impl JobHandler for SucceedingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

/// Stops when cancelled and reports that as a retryable failure.
struct CooperativeHandler;

impl JobHandler for CooperativeHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(JobError::retryable("stopped by shutdown"))
        })
    }
}

struct DeferringHandler;

impl JobHandler for DeferringHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Err(JobError::deferred(
            "circuit open",
            Duration::from_secs(30),
        ))))
    }
}

async fn wait_until(condition: impl Fn() -> bool) {
    tokio::time::timeout(WAIT, async {
        while !condition() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("condition holds in time");
}

fn counter(snapshotter: &Snapshotter, name: &str, label: (&str, &str)) -> Option<u64> {
    snapshotter
        .snapshot()
        .into_vec()
        .into_iter()
        .find_map(|(key, _, _, value)| {
            let key = key.key();
            let matches = key.name() == name
                && key
                    .labels()
                    .any(|item| item.key() == label.0 && item.value() == label.1);
            match value {
                DebugValue::Counter(count) if matches => Some(count),
                _ => None,
            }
        })
}

#[tokio::test]
async fn store_errors_do_not_end_the_runner_and_flip_store_health() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store.fail_next_store_calls(usize::MAX);
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(SucceedingHandler), test_config())
        .expect("valid runner");
    let health = runner.store_health();
    let task = tokio::spawn(runner.run(shutdown.clone()));

    wait_until(|| !health.is_healthy()).await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!task.is_finished(), "store errors must not end the runner");
    store.fail_next_store_calls(0);
    wait_until(|| store.completed.load(Ordering::SeqCst) == 1).await;
    wait_until(|| health.is_healthy()).await;

    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
}

#[tokio::test(start_paused = true)]
async fn shutdown_releases_a_long_job_without_consuming_its_attempt() {
    let store = Arc::new(FakeStore::with_jobs(1));
    let shutdown = ShutdownToken::new(Duration::from_secs(10));
    let runner = WorkerRunner::new(store.clone(), Arc::new(PendingHandler), test_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    wait_until(|| store.remaining() == 0).await;

    shutdown.trigger();
    task.await.expect("runner task").expect("clean drain");

    assert_eq!(
        store.releases(),
        [Release {
            reason: ReleaseReason::Shutdown,
            delay: SignedDuration::ZERO,
        }]
    );
    assert_eq!(store.retried.load(Ordering::SeqCst), 0);
    assert_eq!(store.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_final_attempt_stopped_by_shutdown_is_released_not_failed() {
    let store = Arc::new(FakeStore::with_jobs(0));
    store
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_back(Job {
            attempts: 3,
            max_attempts: 3,
            ..fake_job(0)
        });
    let shutdown = ShutdownToken::new(Duration::from_secs(10));
    let runner = WorkerRunner::new(store.clone(), Arc::new(CooperativeHandler), test_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    wait_until(|| store.remaining() == 0).await;

    shutdown.trigger();
    task.await.expect("runner task").expect("clean drain");

    assert_eq!(
        store.retried.load(Ordering::SeqCst),
        0,
        "no failure recorded"
    );
    assert_eq!(store.releases().len(), 1);
    assert!(
        store
            .releases()
            .iter()
            .all(|release| release.reason == ReleaseReason::Shutdown)
    );
}

#[tokio::test]
async fn a_claim_that_commits_after_shutdown_is_released() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store.block_claim.store(true, Ordering::SeqCst);
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(SucceedingHandler), test_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(WAIT, store.claim_started.notified())
        .await
        .expect("claim starts");

    shutdown.trigger();
    store.claim_release.notify_one();
    task.await.expect("runner task").expect("clean shutdown");

    assert_eq!(store.remaining(), 0, "the claim committed");
    assert_eq!(store.completed.load(Ordering::SeqCst), 0);
    assert_eq!(store.releases().len(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_hanging_store_call_turns_store_health_red() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store.hang_claims.store(true, Ordering::SeqCst);
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(SucceedingHandler), test_config())
        .expect("valid runner");
    let health = runner.store_health();
    let task = tokio::spawn(runner.run(shutdown.clone()));

    wait_until(|| !health.is_healthy()).await;
    assert!(
        !task.is_finished(),
        "a hanging store call must not end the runner"
    );

    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
}

#[tokio::test(start_paused = true)]
async fn a_hanging_claim_at_shutdown_is_abandoned_at_the_drain_budget() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store.block_claim.store(true, Ordering::SeqCst);
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(SucceedingHandler), test_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(WAIT, store.claim_started.notified())
        .await
        .expect("claim starts");

    let started = Instant::now();
    shutdown.trigger();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("run ends within the drain deadline")
        .expect("runner task")
        .expect("clean shutdown");

    assert!(started.elapsed() <= Duration::from_secs(1));
    assert!(store.releases().is_empty());
}

#[tokio::test(start_paused = true)]
async fn running_attempts_renew_their_lease_every_third_of_it() {
    let store = Arc::new(FakeStore::with_jobs(0));
    let handler = Arc::new(BlockingHandler::default());
    let config = WorkerConfig {
        lease_duration: Duration::from_secs(3),
        ..test_config()
    };
    let task = tokio::spawn(process_claimed_job(
        attempt_context(store.clone(), handler.clone(), config),
        fake_job(0),
    ));
    tokio::time::sleep(Duration::from_millis(3_500)).await;
    assert_eq!(store.heartbeats.load(Ordering::SeqCst), 3);

    handler.release.notify_one();
    task.await.expect("attempt task").expect("attempt succeeds");
    assert_eq!(store.completed.load(Ordering::SeqCst), 1);
}

#[tokio::test(start_paused = true)]
async fn a_lost_lease_stops_the_handler_and_counts_an_expiry() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = baukit_telemetry::metrics::set_default_local_recorder(&recorder);
    let store = Arc::new(FakeStore::with_jobs(0));
    store.lease_owned.store(false, Ordering::SeqCst);
    let handler = Arc::new(BlockingHandler::default());
    let config = WorkerConfig {
        lease_duration: Duration::from_secs(3),
        ..test_config()
    };

    process_claimed_job(
        attempt_context(store.clone(), handler.clone(), config),
        fake_job(0),
    )
    .await
    .expect("a lost lease does not fail the runner");

    assert_eq!(handler.active.load(Ordering::SeqCst), 0, "handler dropped");
    assert_eq!(store.completed.load(Ordering::SeqCst), 0);
    assert_eq!(store.retried.load(Ordering::SeqCst), 0);
    assert_eq!(
        counter(
            &snapshotter,
            "worker_lease_expired_total",
            ("queue", "test")
        ),
        Some(1)
    );
}

#[tokio::test]
async fn deferrals_keep_the_attempt_until_the_cap_and_are_counted() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = baukit_telemetry::metrics::set_default_local_recorder(&recorder);
    let store = Arc::new(FakeStore::with_jobs(0));
    let config = test_config();
    for deferrals in [0, config.max_deferrals] {
        process_claimed_job(
            attempt_context(store.clone(), Arc::new(DeferringHandler), config.clone()),
            Job {
                deferrals,
                ..fake_job(0)
            },
        )
        .await
        .expect("deferral recorded");
    }

    assert_eq!(
        store.releases(),
        [
            Release {
                reason: ReleaseReason::Deferred,
                delay: SignedDuration::from_secs(30),
            },
            Release {
                reason: ReleaseReason::DeferredPastCap,
                delay: SignedDuration::from_secs(30),
            },
        ]
    );
    assert_eq!(store.retried.load(Ordering::SeqCst), 0);
    assert_eq!(
        counter(
            &snapshotter,
            "worker_job_deferrals_total",
            ("job_kind", "test.job")
        ),
        Some(2)
    );
}

#[tokio::test]
async fn cancellation_is_counted_as_cancelled_not_failure() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = baukit_telemetry::metrics::set_default_local_recorder(&recorder);
    let store = Arc::new(FakeStore::with_jobs(0));
    store.cancel_requested.store(true, Ordering::SeqCst);

    process_claimed_job(
        attempt_context(store.clone(), Arc::new(PendingHandler), test_config()),
        fake_job(0),
    )
    .await
    .expect("cancellation recorded");

    assert_eq!(store.cancelled.load(Ordering::SeqCst), 1);
    assert_eq!(
        counter(
            &snapshotter,
            "worker_job_runs_total",
            ("outcome", "cancelled")
        ),
        Some(1)
    );
    assert_eq!(
        counter(
            &snapshotter,
            "worker_job_runs_total",
            ("outcome", "failure")
        ),
        None
    );
}

#[tokio::test]
async fn queue_samples_use_the_handler_job_types_and_the_sample_interval() {
    let store = Arc::new(FakeStore::with_jobs(0));
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        store.clone(),
        Arc::new(SucceedingHandler),
        WorkerConfig {
            queue_sample_interval: Duration::from_secs(60),
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    wait_until(|| {
        store
            .claimed_job_types
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
            >= 5
    })
    .await;
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");

    let samples = store
        .sampled_job_types
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert_eq!(samples, [vec!["test.job".to_owned()]]);
}
