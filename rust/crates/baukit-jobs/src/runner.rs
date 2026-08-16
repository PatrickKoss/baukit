use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use baukit_runtime::ShutdownToken;
use chrono::Utc;
use tokio::{
    task::JoinSet,
    time::{Instant, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ClaimedJob, FailureDisposition, JobError, JobStore, RunnerError,
    metrics::{self, FAILURE, RETRY, SUCCESS},
};

/// A boxed future returned by [`JobHandler`].
pub type JobFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Product-owned dispatch and processing for static job types.
pub trait JobHandler: Send + Sync + 'static {
    /// Returns every supported static job identifier.
    ///
    /// These values become the bounded `job` metric label set. They must never
    /// be derived from payload data or runtime configuration.
    fn job_types(&self) -> &'static [&'static str];

    /// Handles one claimed job.
    ///
    /// The future is dropped on timeout or durable cancellation. Long-running
    /// integrations can observe `cancellation` to stop their own child work.
    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>>;
}

/// Cooperative cancellation signal scoped to one job attempt.
#[derive(Clone, Debug)]
pub struct JobCancellation {
    token: CancellationToken,
}

impl JobCancellation {
    fn new() -> Self {
        Self {
            token: CancellationToken::new(),
        }
    }

    /// Returns whether timeout or cancellation has stopped this attempt.
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Waits for timeout or cancellation to stop this attempt.
    pub async fn cancelled(&self) {
        self.token.cancelled().await;
    }

    fn cancel(&self) {
        self.token.cancel();
    }
}

/// Tuning for [`WorkerRunner`].
#[derive(Clone, Debug)]
pub struct WorkerConfig {
    /// Stable identifier written into leases.
    pub worker_id: String,
    /// Static queue identifier used for telemetry.
    pub queue: &'static str,
    /// Maximum number of concurrently running attempts.
    pub concurrency: usize,
    /// Delay between empty-queue polls and queue-age samples.
    pub poll_interval: Duration,
    /// Duration of a claimed job lease.
    pub lease_duration: Duration,
    /// Maximum execution time for one attempt.
    pub job_timeout: Duration,
    /// Frequency at which running attempts check durable cancellation.
    pub cancellation_poll_interval: Duration,
    /// Initial retry delay before exponential growth.
    pub retry_initial: Duration,
    /// Upper bound for exponential retry delay.
    pub retry_max: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            queue: "default",
            concurrency: 5,
            poll_interval: Duration::from_millis(250),
            lease_duration: Duration::from_secs(15 * 60),
            job_timeout: Duration::from_secs(10 * 60),
            cancellation_poll_interval: Duration::from_millis(250),
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(5 * 60),
        }
    }
}

/// Concurrent durable worker loop.
#[derive(Clone)]
pub struct WorkerRunner {
    store: Arc<dyn JobStore>,
    handler: Arc<dyn JobHandler>,
    config: WorkerConfig,
}

impl WorkerRunner {
    /// Validates configuration, registers zero-valued worker metrics, and creates a runner.
    pub fn new(
        store: Arc<dyn JobStore>,
        handler: Arc<dyn JobHandler>,
        config: WorkerConfig,
    ) -> Result<Self, RunnerError> {
        validate_config(&config, handler.job_types())?;
        metrics::initialize(handler.job_types(), config.queue);
        Ok(Self {
            store,
            handler,
            config,
        })
    }

    /// Probes the store's claim-query shape for operations readiness.
    pub async fn ready(&self) -> Result<(), crate::StoreError> {
        self.store.ready().await
    }

    /// Claims and executes jobs until process shutdown begins.
    ///
    /// New claims stop immediately after shutdown is observed. Already running
    /// attempts drain in the `JoinSet`; `TaskSupervisor` bounds that drain using
    /// the same [`ShutdownToken`] deadline.
    pub async fn run(self, shutdown: ShutdownToken) -> Result<(), RunnerError> {
        let mut running = JoinSet::new();
        loop {
            drain_finished(&mut running).await?;
            if shutdown.is_cancelled() {
                break;
            }

            let now = Utc::now();
            let age = self.store.oldest_pending_age(now).await?;
            metrics::set_queue_age(self.config.queue, age);
            while running.len() < self.config.concurrency && !shutdown.is_cancelled() {
                let claimed = tokio::select! {
                    () = shutdown.cancelled() => None,
                    result = self.store.claim(
                        &self.config.worker_id,
                        now,
                        self.config.lease_duration,
                    ) => result?,
                };
                let Some(job) = claimed else {
                    break;
                };
                spawn_attempt(
                    &mut running,
                    Arc::clone(&self.store),
                    Arc::clone(&self.handler),
                    self.config.clone(),
                    job,
                );
            }

            if running.is_empty() {
                tokio::select! {
                    () = shutdown.cancelled() => {},
                    () = tokio::time::sleep(self.config.poll_interval) => {},
                }
            } else {
                tokio::select! {
                    () = shutdown.cancelled() => {},
                    () = tokio::time::sleep(self.config.poll_interval) => {},
                    result = running.join_next() => {
                        if let Some(result) = result {
                            result??;
                        }
                    }
                }
            }
        }

        while let Some(result) = running.join_next().await {
            result??;
        }
        Ok(())
    }
}

fn spawn_attempt(
    running: &mut JoinSet<Result<(), RunnerError>>,
    store: Arc<dyn JobStore>,
    handler: Arc<dyn JobHandler>,
    config: WorkerConfig,
    job: ClaimedJob,
) {
    running.spawn(async move { process_claimed_job(store, handler, config, job).await });
}

async fn drain_finished(running: &mut JoinSet<Result<(), RunnerError>>) -> Result<(), RunnerError> {
    while let Some(result) = running.try_join_next() {
        result??;
    }
    Ok(())
}

enum AttemptResult {
    Handled(Result<(), JobError>),
    Cancelled,
}

async fn process_claimed_job(
    store: Arc<dyn JobStore>,
    handler: Arc<dyn JobHandler>,
    config: WorkerConfig,
    job: ClaimedJob,
) -> Result<(), RunnerError> {
    let started = Instant::now();
    let cancellation = JobCancellation::new();
    let future = handler.handle(&job, cancellation.clone());
    tokio::pin!(future);
    let timeout = tokio::time::sleep(config.job_timeout);
    tokio::pin!(timeout);
    let mut cancellation_poll = tokio::time::interval(config.cancellation_poll_interval);
    cancellation_poll.set_missed_tick_behavior(MissedTickBehavior::Skip);
    cancellation_poll.tick().await;

    let result = loop {
        tokio::select! {
            result = &mut future => break AttemptResult::Handled(result),
            () = &mut timeout => {
                cancellation.cancel();
                break AttemptResult::Handled(Err(JobError::retryable("job timed out")));
            }
            _ = cancellation_poll.tick() => {
                if store.cancellation_requested(job.id).await? {
                    cancellation.cancel();
                    break AttemptResult::Cancelled;
                }
            }
        }
    };

    match result {
        AttemptResult::Cancelled => {
            ensure_transition(
                store.cancel(job.id, &config.worker_id, Utc::now()).await?,
                job.id,
            )?;
            metrics::record(
                &job.job_type,
                handler.job_types(),
                FAILURE,
                started.elapsed(),
            );
        }
        AttemptResult::Handled(Ok(())) => {
            let now = Utc::now();
            if store.complete(job.id, &config.worker_id, now).await? {
                metrics::record(
                    &job.job_type,
                    handler.job_types(),
                    SUCCESS,
                    started.elapsed(),
                );
            } else if store.cancellation_requested(job.id).await?
                && store.cancel(job.id, &config.worker_id, now).await?
            {
                metrics::record(
                    &job.job_type,
                    handler.job_types(),
                    FAILURE,
                    started.elapsed(),
                );
            } else {
                return Err(RunnerError::LeaseLost { job_id: job.id });
            }
        }
        AttemptResult::Handled(Err(error)) => {
            let now = Utc::now();
            let delay = error
                .retry_after()
                .unwrap_or_else(|| retry_delay(&config, job.attempts));
            let retry_at = add_duration(now, delay)?;
            if let Some(disposition) = store
                .record_failure(
                    job.id,
                    &config.worker_id,
                    error.is_retryable(),
                    retry_at,
                    &error.to_string(),
                    now,
                )
                .await?
            {
                let outcome = match disposition {
                    FailureDisposition::Retry => RETRY,
                    FailureDisposition::Failed => FAILURE,
                };
                metrics::record(
                    &job.job_type,
                    handler.job_types(),
                    outcome,
                    started.elapsed(),
                );
            } else if store.cancellation_requested(job.id).await?
                && store.cancel(job.id, &config.worker_id, now).await?
            {
                metrics::record(
                    &job.job_type,
                    handler.job_types(),
                    FAILURE,
                    started.elapsed(),
                );
            } else {
                return Err(RunnerError::LeaseLost { job_id: job.id });
            }
        }
    }
    Ok(())
}

fn retry_delay(config: &WorkerConfig, attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(31);
    config
        .retry_initial
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(config.retry_max)
}

fn add_duration(
    timestamp: chrono::DateTime<Utc>,
    duration: Duration,
) -> Result<chrono::DateTime<Utc>, RunnerError> {
    let duration = chrono::Duration::from_std(duration)
        .map_err(|_| RunnerError::InvalidConfig("retry delay is out of range".to_owned()))?;
    timestamp
        .checked_add_signed(duration)
        .ok_or_else(|| RunnerError::InvalidConfig("retry deadline is out of range".to_owned()))
}

fn ensure_transition(updated: bool, job_id: Uuid) -> Result<(), RunnerError> {
    if updated {
        Ok(())
    } else {
        Err(RunnerError::LeaseLost { job_id })
    }
}

fn validate_config(
    config: &WorkerConfig,
    job_types: &'static [&'static str],
) -> Result<(), RunnerError> {
    if config.worker_id.trim().is_empty() {
        return Err(RunnerError::InvalidConfig(
            "worker_id must not be empty".to_owned(),
        ));
    }
    if config.queue.trim().is_empty() {
        return Err(RunnerError::InvalidConfig(
            "queue must not be empty".to_owned(),
        ));
    }
    if config.concurrency == 0 {
        return Err(RunnerError::InvalidConfig(
            "concurrency must be greater than zero".to_owned(),
        ));
    }
    for (name, duration) in [
        ("poll_interval", config.poll_interval),
        ("lease_duration", config.lease_duration),
        ("job_timeout", config.job_timeout),
        (
            "cancellation_poll_interval",
            config.cancellation_poll_interval,
        ),
        ("retry_initial", config.retry_initial),
        ("retry_max", config.retry_max),
    ] {
        if duration.is_zero() {
            return Err(RunnerError::InvalidConfig(format!(
                "{name} must be greater than zero"
            )));
        }
    }
    if config.retry_initial > config.retry_max {
        return Err(RunnerError::InvalidConfig(
            "retry_initial must not exceed retry_max".to_owned(),
        ));
    }
    if config.lease_duration <= config.job_timeout {
        return Err(RunnerError::InvalidConfig(
            "lease_duration must exceed job_timeout".to_owned(),
        ));
    }
    if job_types.is_empty() || job_types.iter().any(|job| job.trim().is_empty()) {
        return Err(RunnerError::InvalidConfig(
            "job_types must contain non-empty static identifiers".to_owned(),
        ));
    }
    for (index, job) in job_types.iter().enumerate() {
        if job_types[..index].contains(job) {
            return Err(RunnerError::InvalidConfig(format!(
                "duplicate job type `{job}`"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Mutex,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
    };

    use chrono::TimeDelta;
    use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
    use serde_json::json;
    use tokio::sync::Notify;

    use super::*;
    use crate::{EnqueueOutcome, Job, JobStatus, NewJob, StoreError, StoreFuture};

    #[test]
    fn retry_delay_is_exponential_and_bounded() {
        let config = WorkerConfig {
            retry_initial: Duration::from_secs(5),
            retry_max: Duration::from_secs(60),
            ..WorkerConfig::default()
        };
        assert_eq!(retry_delay(&config, 1), Duration::from_secs(5));
        assert_eq!(retry_delay(&config, 2), Duration::from_secs(10));
        assert_eq!(retry_delay(&config, 5), Duration::from_secs(60));
        assert_eq!(retry_delay(&config, u32::MAX), Duration::from_secs(60));
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
        let store = Arc::new(FakeStore::with_jobs(0));
        process_claimed_job(
            store.clone(),
            Arc::new(RetryAfterHandler),
            test_config(),
            fake_job(0),
        )
        .await
        .expect("attempt recorded");

        assert_eq!(
            *store
                .recorded_retry_delay
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            Some(chrono::Duration::seconds(42))
        );
    }

    fn test_config() -> WorkerConfig {
        WorkerConfig {
            worker_id: "test-worker".to_owned(),
            queue: "test",
            concurrency: 1,
            poll_interval: Duration::from_millis(5),
            lease_duration: Duration::from_secs(120),
            job_timeout: Duration::from_secs(60),
            cancellation_poll_interval: Duration::from_millis(5),
            retry_initial: Duration::from_millis(5),
            retry_max: Duration::from_secs(1),
        }
    }

    struct FakeStore {
        jobs: Mutex<VecDeque<Job>>,
        completed: AtomicUsize,
        retried: AtomicUsize,
        cancelled: AtomicUsize,
        cancel_requested: AtomicBool,
        transition: Notify,
        recorded_retry_delay: Mutex<Option<chrono::Duration>>,
    }

    impl FakeStore {
        fn with_jobs(count: usize) -> Self {
            Self {
                jobs: Mutex::new((0..count).map(fake_job).collect()),
                completed: AtomicUsize::new(0),
                retried: AtomicUsize::new(0),
                cancelled: AtomicUsize::new(0),
                cancel_requested: AtomicBool::new(false),
                transition: Notify::new(),
                recorded_retry_delay: Mutex::new(None),
            }
        }

        fn remaining(&self) -> usize {
            self.jobs
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .len()
        }
    }

    impl JobStore for FakeStore {
        fn enqueue(&self, _job: NewJob) -> StoreFuture<'_, Result<EnqueueOutcome, StoreError>> {
            Box::pin(async {
                Err(StoreError::InvalidInput(
                    "enqueue is unused by this fake".to_owned(),
                ))
            })
        }

        fn claim<'a>(
            &'a self,
            _worker_id: &'a str,
            _now: chrono::DateTime<Utc>,
            _lease_for: Duration,
        ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>> {
            Box::pin(async move {
                Ok(self
                    .jobs
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .pop_front())
            })
        }

        fn complete<'a>(
            &'a self,
            _job_id: Uuid,
            _worker_id: &'a str,
            _now: chrono::DateTime<Utc>,
        ) -> StoreFuture<'a, Result<bool, StoreError>> {
            Box::pin(async move {
                self.completed.fetch_add(1, Ordering::SeqCst);
                self.transition.notify_one();
                Ok(true)
            })
        }

        fn record_failure<'a>(
            &'a self,
            _job_id: Uuid,
            _worker_id: &'a str,
            _retryable: bool,
            retry_at: chrono::DateTime<Utc>,
            _error: &'a str,
            now: chrono::DateTime<Utc>,
        ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
            Box::pin(async move {
                *self
                    .recorded_retry_delay
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(retry_at - now);
                self.retried.fetch_add(1, Ordering::SeqCst);
                self.transition.notify_one();
                Ok(Some(FailureDisposition::Retry))
            })
        }

        fn cancellation_requested(
            &self,
            _job_id: Uuid,
        ) -> StoreFuture<'_, Result<bool, StoreError>> {
            Box::pin(async move { Ok(self.cancel_requested.load(Ordering::SeqCst)) })
        }

        fn request_cancellation(
            &self,
            _job_id: Uuid,
            _now: chrono::DateTime<Utc>,
        ) -> StoreFuture<'_, Result<bool, StoreError>> {
            Box::pin(async { Ok(false) })
        }

        fn cancel<'a>(
            &'a self,
            _job_id: Uuid,
            _worker_id: &'a str,
            _now: chrono::DateTime<Utc>,
        ) -> StoreFuture<'a, Result<bool, StoreError>> {
            Box::pin(async move {
                self.cancelled.fetch_add(1, Ordering::SeqCst);
                self.transition.notify_one();
                Ok(true)
            })
        }

        fn oldest_pending_age(
            &self,
            _now: chrono::DateTime<Utc>,
        ) -> StoreFuture<'_, Result<Duration, StoreError>> {
            Box::pin(async { Ok(Duration::ZERO) })
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

    fn fake_job(sequence: usize) -> Job {
        let now = Utc::now();
        Job {
            id: Uuid::now_v7(),
            job_type: "test.job".to_owned(),
            payload: json!({"sequence": sequence}),
            status: JobStatus::Running,
            attempts: 1,
            max_attempts: 3,
            run_after: now,
            locked_by: Some("test-worker".to_owned()),
            locked_until: Some(now + TimeDelta::minutes(1)),
            idempotency_key: None,
            last_error: None,
            failure_reason: None,
            cancel_requested_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}
