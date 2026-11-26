use std::{
    collections::VecDeque,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

use jiff::SignedDuration;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use serde_json::json;
use tokio::sync::Notify;

use super::{attempt::retry_delay, *};
use crate::{
    EnqueueOutcome, FailureDisposition, Job, JobLease, JobStatus, NewJob, QueueDepth,
    ReleaseReason, StoreError, StoreFuture,
};

mod lifecycle;
mod reliability;

#[test]
fn retry_delay_is_full_jitter_under_an_exponential_bound() {
    let config = WorkerConfig {
        retry_initial: Duration::from_secs(5),
        retry_max: Duration::from_secs(60),
        ..WorkerConfig::default()
    };
    assert_eq!(retry_delay(&config, 1, 0.5), Duration::from_millis(2500));
    assert_eq!(retry_delay(&config, 2, 0.5), Duration::from_secs(5));
    assert_eq!(retry_delay(&config, 5, 0.5), Duration::from_secs(30));
    assert_eq!(retry_delay(&config, u32::MAX, 0.5), Duration::from_secs(30));
    assert_eq!(retry_delay(&config, 3, 0.0), Duration::ZERO);
}

#[test]
fn configuration_rejects_duplicate_job_labels() {
    let error = validate_config(&WorkerConfig::default(), &["sync", "sync"])
        .expect_err("duplicate label must fail");
    assert!(error.to_string().contains("duplicate job type"));
}

#[test]
fn runner_initial_metrics_pass_worker_conformance() {
    let recorder = PrometheusBuilder::new()
        .set_buckets_for_metric(
            Matcher::Full("worker_job_duration_seconds".to_owned()),
            baukit_telemetry::WORKER_DURATION_BUCKETS,
        )
        .expect("worker buckets are valid")
        .build_recorder();
    let handle = recorder.handle();
    let _guard = baukit_telemetry::metrics::set_default_local_recorder(&recorder);
    baukit_telemetry::metrics::gauge!(
        "build_info",
        "version" => "0.2.0",
        "commit" => "test",
        "rust_version" => "1.95"
    )
    .set(1.0);

    WorkerRunner::new(
        Arc::new(FakeStore::with_jobs(0)),
        Arc::new(PendingHandler),
        test_config(),
    )
    .expect("valid runner");

    baukit_test::check_metrics_conformance_with_options(
        handle.render(),
        baukit_test::MetricsConformanceOptions::new().require_worker_metrics(),
    )
    .expect("WorkerRunner startup output conforms to telemetry-spec section 2.4");
}

#[tokio::test]
async fn shutdown_stops_claiming_and_drains_the_join_set() {
    let store = Arc::new(FakeStore::with_jobs(3));
    let handler = Arc::new(BlockingHandler::default());
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        store.clone(),
        handler.clone(),
        WorkerConfig {
            concurrency: 2,
            poll_interval: Duration::from_millis(5),
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));

    tokio::time::timeout(Duration::from_secs(1), async {
        while handler.maximum.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("two attempts run concurrently");
    shutdown.trigger();
    assert!(!task.is_finished(), "running attempts must drain");
    handler.release.notify_waiters();
    task.await.expect("runner task").expect("clean drain");

    assert_eq!(store.completed.load(Ordering::SeqCst), 2);
    assert_eq!(store.remaining(), 1, "shutdown prevents another claim");
}

#[tokio::test]
async fn each_claim_uses_a_fresh_time_for_jobs_enqueued_during_the_poll_loop() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store.enqueue_during_claim.store(true, Ordering::SeqCst);
    let handler = Arc::new(BlockingHandler::default());
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        store.clone(),
        handler.clone(),
        WorkerConfig {
            concurrency: 2,
            poll_interval: Duration::from_secs(60),
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));

    tokio::time::timeout(Duration::from_secs(1), async {
        while handler.maximum.load(Ordering::SeqCst) < 2 && !task.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both jobs claimed in one poll loop");
    assert!(!task.is_finished(), "the second claim must not fail");
    shutdown.trigger();
    handler.release.notify_waiters();
    task.await.expect("runner task").expect("clean drain");
    assert_eq!(store.completed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn every_claim_passes_the_handler_job_types() {
    let store = Arc::new(FakeStore::with_jobs(1));
    store
        .jobs
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push_front(Job {
            job_type: "other.job".to_owned(),
            ..fake_job(99)
        });
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(store.clone(), Arc::new(RetryAfterHandler), test_config())
        .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(Duration::from_secs(1), store.transition.notified())
        .await
        .expect("handled job transition recorded");
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");

    let claims = store
        .claimed_job_types
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    assert!(!claims.is_empty());
    assert!(claims.iter().all(|types| types == &["test.job"]));
    assert_eq!(store.remaining(), 1, "the undeclared job type stays queued");
}

#[tokio::test]
async fn timeout_records_a_retry_and_runner_poll_honors_cancellation() {
    let timeout_store = Arc::new(FakeStore::with_jobs(1));
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        timeout_store.clone(),
        Arc::new(PendingHandler),
        WorkerConfig {
            job_timeout: Duration::from_millis(10),
            cancellation_poll_interval: Duration::from_secs(1),
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(Duration::from_secs(1), timeout_store.transition.notified())
        .await
        .expect("timeout transition recorded");
    assert_eq!(timeout_store.retried.load(Ordering::SeqCst), 1);
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");

    let cancel_store = Arc::new(FakeStore::with_jobs(1));
    cancel_store.cancel_requested.store(true, Ordering::SeqCst);
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let runner = WorkerRunner::new(
        cancel_store.clone(),
        Arc::new(PendingHandler),
        WorkerConfig {
            job_timeout: Duration::from_secs(1),
            cancellation_poll_interval: Duration::from_millis(5),
            ..test_config()
        },
    )
    .expect("valid runner");
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(Duration::from_secs(1), cancel_store.transition.notified())
        .await
        .expect("cancellation transition recorded");
    assert_eq!(cancel_store.cancelled.load(Ordering::SeqCst), 1);
    shutdown.trigger();
    task.await.expect("runner task").expect("clean shutdown");
}

#[tokio::test]
async fn provider_retry_delay_overrides_exponential_backoff() {
    let config = WorkerConfig {
        retry_max: Duration::from_secs(60),
        ..test_config()
    };
    assert_eq!(
        recorded_retry_delay(config).await,
        Some(SignedDuration::from_secs(42))
    );
}

#[tokio::test]
async fn provider_retry_delay_is_capped_at_retry_max() {
    assert_eq!(
        recorded_retry_delay(test_config()).await,
        Some(SignedDuration::from_secs(1))
    );
}

async fn recorded_retry_delay(config: WorkerConfig) -> Option<SignedDuration> {
    let store = Arc::new(FakeStore::with_jobs(0));
    process_claimed_job(
        attempt_context(store.clone(), Arc::new(RetryAfterHandler), config),
        fake_job(0),
    )
    .await
    .expect("attempt recorded");
    *store
        .recorded_retry_delay
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[tokio::test]
async fn handler_owned_transactional_completion_is_not_completed_twice() {
    let store = Arc::new(FakeStore::with_jobs(0));
    process_claimed_job(
        attempt_context(
            store.clone(),
            Arc::new(TransactionalCompletionHandler),
            test_config(),
        ),
        fake_job(0),
    )
    .await
    .expect("transactionally completed attempt succeeds");

    assert_eq!(store.completed.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_pending_cancellation_query_does_not_stop_the_handler() {
    let store = Arc::new(FakeStore::with_jobs(0));
    store.block_cancellation_poll.store(true, Ordering::SeqCst);
    let handler = Arc::new(BlockingHandler::default());
    let task = tokio::spawn(process_claimed_job(
        attempt_context(store.clone(), handler.clone(), test_config()),
        fake_job(0),
    ));
    tokio::time::timeout(
        Duration::from_secs(1),
        store.cancellation_poll_started.notified(),
    )
    .await
    .expect("cancellation query starts");
    handler.release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("handler completes while the cancellation query is pending")
        .expect("attempt task")
        .expect("attempt succeeds");
    assert_eq!(store.completed.load(Ordering::SeqCst), 1);
    assert_eq!(handler.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(handler.active.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn a_pending_cancellation_query_does_not_stop_the_timeout() {
    let store = Arc::new(FakeStore::with_jobs(0));
    store.block_cancellation_poll.store(true, Ordering::SeqCst);
    let handler = Arc::new(BlockingHandler::default());
    let task = tokio::spawn(process_claimed_job(
        attempt_context(
            store.clone(),
            handler.clone(),
            WorkerConfig {
                job_timeout: Duration::from_millis(20),
                ..test_config()
            },
        ),
        fake_job(0),
    ));
    tokio::time::timeout(
        Duration::from_secs(1),
        store.cancellation_poll_started.notified(),
    )
    .await
    .expect("cancellation query starts");
    assert_eq!(handler.maximum.load(Ordering::SeqCst), 1);
    tokio::time::timeout(Duration::from_secs(1), async {
        while handler.active.load(Ordering::SeqCst) != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("timeout drops the handler while the cancellation query is pending");
    store.block_cancellation_poll.store(false, Ordering::SeqCst);
    store.cancellation_poll_release.notify_one();
    tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("timeout outcome is recorded")
        .expect("attempt task")
        .expect("attempt succeeds");
    assert_eq!(store.retried.load(Ordering::SeqCst), 1);
    assert_eq!(store.completed.load(Ordering::SeqCst), 0);
}

fn test_config() -> WorkerConfig {
    WorkerConfig {
        worker_id: "test-worker".to_owned(),
        queue: "test",
        concurrency: 1,
        poll_interval: Duration::from_millis(5),
        idle_poll_max: Duration::from_millis(5),
        lease_duration: Duration::from_secs(120),
        job_timeout: Duration::from_secs(60),
        cancellation_poll_interval: Duration::from_millis(5),
        retry_initial: Duration::from_millis(5),
        retry_max: Duration::from_secs(1),
        max_per_key: None,
        max_deferrals: 2,
        queue_sample_interval: Duration::from_millis(5),
        release_reserve: Duration::from_millis(100),
        store_call_timeout: Duration::from_millis(200),
    }
}

fn attempt_context(
    store: Arc<FakeStore>,
    handler: Arc<dyn JobHandler>,
    config: WorkerConfig,
) -> Arc<AttemptContext> {
    Arc::new(AttemptContext {
        store,
        handler,
        config,
        drain: CancellationToken::new(),
        release_now: CancellationToken::new(),
        jitter: SharedJitter::from_entropy(),
        recurring: Vec::new(),
    })
}

struct FakeStore {
    jobs: Mutex<VecDeque<Job>>,
    completed: AtomicUsize,
    retried: AtomicUsize,
    cancelled: AtomicUsize,
    cancel_requested: AtomicBool,
    block_cancellation_poll: AtomicBool,
    cancellation_poll_started: Notify,
    cancellation_poll_release: Notify,
    transition: Notify,
    recorded_retry_delay: Mutex<Option<jiff::SignedDuration>>,
    claimed_job_types: Mutex<Vec<Vec<String>>>,
    enqueue_during_claim: AtomicBool,
    failing_store_calls: AtomicUsize,
    heartbeats: AtomicUsize,
    lease_owned: AtomicBool,
    releases: Mutex<Vec<Release>>,
    sampled_job_types: Mutex<Vec<Vec<String>>>,
    block_claim: AtomicBool,
    hang_claims: AtomicBool,
    claim_started: Notify,
    claim_release: Notify,
    enqueued: Mutex<Vec<NewJob>>,
    enqueue_status: Mutex<JobStatus>,
    failure_disposition: Mutex<FailureDisposition>,
    max_per_key_seen: Mutex<Vec<Option<std::num::NonZeroU32>>>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Release {
    reason: ReleaseReason,
    delay: SignedDuration,
}

impl FakeStore {
    fn with_jobs(count: usize) -> Self {
        Self {
            jobs: Mutex::new((0..count).map(fake_job).collect()),
            completed: AtomicUsize::new(0),
            retried: AtomicUsize::new(0),
            cancelled: AtomicUsize::new(0),
            cancel_requested: AtomicBool::new(false),
            block_cancellation_poll: AtomicBool::new(false),
            cancellation_poll_started: Notify::new(),
            cancellation_poll_release: Notify::new(),
            transition: Notify::new(),
            recorded_retry_delay: Mutex::new(None),
            claimed_job_types: Mutex::new(Vec::new()),
            enqueue_during_claim: AtomicBool::new(false),
            failing_store_calls: AtomicUsize::new(0),
            heartbeats: AtomicUsize::new(0),
            lease_owned: AtomicBool::new(true),
            releases: Mutex::new(Vec::new()),
            sampled_job_types: Mutex::new(Vec::new()),
            block_claim: AtomicBool::new(false),
            hang_claims: AtomicBool::new(false),
            claim_started: Notify::new(),
            claim_release: Notify::new(),
            enqueued: Mutex::new(Vec::new()),
            enqueue_status: Mutex::new(JobStatus::Pending),
            failure_disposition: Mutex::new(FailureDisposition::Retry),
            max_per_key_seen: Mutex::new(Vec::new()),
        }
    }

    fn fail_next_store_calls(&self, count: usize) {
        self.failing_store_calls.store(count, Ordering::SeqCst);
    }

    fn take_failure(&self) -> Result<(), StoreError> {
        loop {
            let left = self.failing_store_calls.load(Ordering::SeqCst);
            let Some(next) = left.checked_sub(1) else {
                return Ok(());
            };
            if self
                .failing_store_calls
                .compare_exchange(left, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Err(StoreError::InvalidData("injected store outage".to_owned()));
            }
        }
    }

    fn releases(&self) -> Vec<Release> {
        self.releases
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn remaining(&self) -> usize {
        self.jobs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    }
}

impl JobStore for FakeStore {
    fn enqueue(&self, job: NewJob) -> StoreFuture<'_, Result<EnqueueOutcome, StoreError>> {
        Box::pin(async move {
            let status = *lock(&self.enqueue_status);
            let stored = Job {
                id: job.id,
                job_type: job.job_type.clone(),
                payload: job.payload.clone(),
                status,
                run_after: job.run_after,
                idempotency_key: job.idempotency_key.clone(),
                ..fake_job(0)
            };
            lock(&self.enqueued).push(job);
            self.transition.notify_one();
            Ok(EnqueueOutcome {
                job: stored,
                created: status == JobStatus::Pending,
            })
        })
    }

    fn claim_with_key_limit<'a>(
        &'a self,
        _worker_id: &'a str,
        job_types: &'a [&'a str],
        now: jiff::Timestamp,
        _lease_for: Duration,
        max_per_key: Option<std::num::NonZeroU32>,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>> {
        Box::pin(async move {
            self.take_failure()?;
            lock(&self.max_per_key_seen).push(max_per_key);
            if self.hang_claims.load(Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            if self.block_claim.swap(false, Ordering::SeqCst) {
                self.claim_started.notify_one();
                self.claim_release.notified().await;
            }
            self.claimed_job_types
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(job_types.iter().map(|job| (*job).to_owned()).collect());
            let claimed = {
                let mut jobs = self
                    .jobs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                let position = jobs
                    .iter()
                    .position(|job| job_types.contains(&job.job_type.as_str()));
                position.and_then(|position| jobs.remove(position))
            };
            if claimed.as_ref().is_some_and(|job| job.created_at > now) {
                return Err(StoreError::InvalidInput(
                    "updated_at would precede created_at".to_owned(),
                ));
            }
            if self.enqueue_during_claim.swap(false, Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(2)).await;
                self.jobs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push_back(fake_job(100));
            }
            Ok(claimed)
        })
    }

    fn complete<'a>(
        &'a self,
        _lease: &'a JobLease,
        _now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            self.completed.fetch_add(1, Ordering::SeqCst);
            self.transition.notify_one();
            Ok(true)
        })
    }

    fn record_failure<'a>(
        &'a self,
        _lease: &'a JobLease,
        _retryable: bool,
        retry_at: jiff::Timestamp,
        _error: &'a str,
        now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
        Box::pin(async move {
            *self
                .recorded_retry_delay
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(retry_at - now);
            self.retried.fetch_add(1, Ordering::SeqCst);
            self.transition.notify_one();
            Ok(Some(*lock(&self.failure_disposition)))
        })
    }

    fn cancellation_requested(&self, _job_id: Uuid) -> StoreFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            if self.block_cancellation_poll.load(Ordering::SeqCst) {
                self.cancellation_poll_started.notify_one();
                self.cancellation_poll_release.notified().await;
            }
            Ok(self.cancel_requested.load(Ordering::SeqCst))
        })
    }

    fn request_cancellation(
        &self,
        _job_id: Uuid,
        _now: jiff::Timestamp,
    ) -> StoreFuture<'_, Result<bool, StoreError>> {
        Box::pin(async { Ok(false) })
    }

    fn cancel<'a>(
        &'a self,
        _lease: &'a JobLease,
        _now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            self.cancelled.fetch_add(1, Ordering::SeqCst);
            self.transition.notify_one();
            Ok(true)
        })
    }

    fn heartbeat<'a>(
        &'a self,
        _lease: &'a JobLease,
        _lease_for: Duration,
        _now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            self.heartbeats.fetch_add(1, Ordering::SeqCst);
            Ok(self.lease_owned.load(Ordering::SeqCst))
        })
    }

    fn release<'a>(
        &'a self,
        _lease: &'a JobLease,
        run_after: jiff::Timestamp,
        reason: ReleaseReason,
        now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
        Box::pin(async move {
            self.releases
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(Release {
                    reason,
                    delay: run_after - now,
                });
            self.transition.notify_one();
            Ok(Some(FailureDisposition::Retry))
        })
    }

    fn oldest_pending_age<'a>(
        &'a self,
        job_types: &'a [&'a str],
        _now: jiff::Timestamp,
    ) -> StoreFuture<'a, Result<Duration, StoreError>> {
        Box::pin(async move {
            self.take_failure()?;
            self.sampled_job_types
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(job_types.iter().map(|job| (*job).to_owned()).collect());
            Ok(Duration::ZERO)
        })
    }

    fn queue_depth<'a>(
        &'a self,
        _job_types: &'a [&'a str],
    ) -> StoreFuture<'a, Result<QueueDepth, StoreError>> {
        Box::pin(async { Ok(QueueDepth::default()) })
    }

    fn ready(&self) -> StoreFuture<'_, Result<(), StoreError>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Default)]
struct BlockingHandler {
    active: AtomicUsize,
    maximum: AtomicUsize,
    release: Notify,
}

impl JobHandler for BlockingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            let _guard = ActiveGuard(&self.active);
            self.release.notified().await;
            Ok(())
        })
    }
}

struct ActiveGuard<'a>(&'a AtomicUsize);

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct PendingHandler;

impl JobHandler for PendingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::pending())
    }
}

struct RetryAfterHandler;

impl JobHandler for RetryAfterHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Err(JobError::retryable_after(
            "provider rate limited",
            Duration::from_secs(42),
        ))))
    }
}

struct TransactionalCompletionHandler;

impl JobHandler for TransactionalCompletionHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["test.job"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            assert_eq!(cancellation.worker_id(), "test-worker");
            cancellation.mark_completed_in_transaction();
            Ok(())
        })
    }
}

fn fake_job(sequence: usize) -> Job {
    let now = Timestamp::now();
    Job {
        id: Uuid::now_v7(),
        job_type: "test.job".to_owned(),
        payload: json!({"sequence": sequence}),
        status: JobStatus::Running,
        attempts: 1,
        max_attempts: 3,
        run_after: now,
        locked_by: Some("test-worker".to_owned()),
        locked_until: Some(now + SignedDuration::from_mins(1)),
        idempotency_key: None,
        dedupe_key: None,
        pending_dedupe_key: None,
        last_error: None,
        failure_reason: None,
        cancel_requested_at: None,
        deferrals: 0,
        concurrency_key: None,
        created_at: now,
        updated_at: now,
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
