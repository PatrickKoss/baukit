use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use baukit_runtime::ShutdownToken;
use jiff::{SignedDuration, Timestamp};
use tokio::{
    task::JoinSet,
    time::{Instant, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ClaimedJob, FailureDisposition, JobError, JobLease, JobStore, RunnerError,
    metrics::{self, FAILURE, RETRY, SUCCESS},
};

mod config;

use config::validate_config;

/// A boxed future returned by [`JobHandler`].
pub type JobFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Product-owned dispatch and processing for static job types.
pub trait JobHandler: Send + Sync + 'static {
    /// Returns every supported static job identifier.
    ///
    /// These values become the bounded `job_kind` metric label set. They must never
    /// be derived from payload data or runtime configuration.
    fn job_types(&self) -> &'static [&'static str];

    /// Handles one claimed job.
    ///
    /// The future is dropped on timeout or durable cancellation before the runner
    /// records the outcome. Long-running integrations can observe `cancellation`
    /// to stop their own child work.
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
    lease: Arc<JobLease>,
    completed_in_transaction: Arc<AtomicBool>,
}

impl JobCancellation {
    fn new(lease: JobLease) -> Self {
        Self {
            token: CancellationToken::new(),
            lease: Arc::new(lease),
            completed_in_transaction: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the lease owner required by `PostgresJobStore::complete_in_transaction`.
    pub fn worker_id(&self) -> &str {
        &self.lease.worker_id
    }

    /// Returns this attempt's lease for `PostgresJobStore::complete_lease_in_transaction`.
    pub fn lease(&self) -> &JobLease {
        &self.lease
    }

    /// Reports that the handler committed its result and successful job transition together.
    ///
    /// Call this only after the transaction containing `complete_in_transaction`
    /// commits successfully, and return success immediately afterward. The runner
    /// then records success without attempting a second completion transition.
    pub fn mark_completed_in_transaction(&self) {
        self.completed_in_transaction.store(true, Ordering::Release);
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

    fn was_completed_in_transaction(&self) -> bool {
        self.completed_in_transaction.load(Ordering::Acquire)
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
    ///
    /// # Errors
    ///
    /// [`RunnerError::InvalidConfig`] for an empty worker id or queue, zero concurrency or
    /// duration, `retry_initial` above `retry_max`, a lease not longer than the job timeout,
    /// or empty or duplicate job types.
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
    ///
    /// # Errors
    ///
    /// Whatever the store's [`JobStore::ready`] returns. [`PostgresJobStore`](crate::PostgresJobStore)
    /// returns [`crate::StoreError::Database`] when the probe query fails and
    /// [`crate::StoreError::Task`] when its owned task panics or is aborted.
    pub async fn ready(&self) -> Result<(), crate::StoreError> {
        self.store.ready().await
    }

    /// Claims and executes jobs until process shutdown begins.
    ///
    /// Every claim passes the handler's [`JobHandler::job_types`], so the runner
    /// never claims a job type its handler does not declare.
    ///
    /// New claims stop immediately after shutdown is observed. Claim
    /// transactions run in owned tasks. If shutdown or caller cancellation
    /// abandons a claim, it finishes and the job recovers after lease expiry.
    /// Running attempts drain in the `JoinSet`; `TaskSupervisor` bounds that
    /// drain using the same [`ShutdownToken`] deadline.
    ///
    /// # Errors
    ///
    /// [`RunnerError::Store`] when a store call fails, [`RunnerError::Task`] when a claim or
    /// attempt task panics or is aborted, and [`RunnerError::LeaseLost`] when an attempt can no
    /// longer persist its outcome because another worker owns the job, and
    /// [`RunnerError::InvalidConfig`] when a retry deadline overflows the timestamp range.
    pub async fn run(self, shutdown: ShutdownToken) -> Result<(), RunnerError> {
        let mut running = JoinSet::new();
        loop {
            drain_finished(&mut running)?;
            if shutdown.is_cancelled() {
                break;
            }

            let age = self.store.oldest_pending_age(Timestamp::now()).await?;
            metrics::set_queue_age(self.config.queue, age);
            while running.len() < self.config.concurrency && !shutdown.is_cancelled() {
                let store = Arc::clone(&self.store);
                let worker_id = self.config.worker_id.clone();
                let job_types = self.handler.job_types();
                let lease_duration = self.config.lease_duration;
                // Jobs enqueued during this loop must not be claimed before their creation time.
                let claim = tokio::spawn(async move {
                    store
                        .claim(&worker_id, job_types, Timestamp::now(), lease_duration)
                        .await
                });
                let claimed = tokio::select! {
                    () = shutdown.cancelled() => None,
                    result = claim => claim_or_log(result),
                };
                let Some(job) = claimed else {
                    break;
                };
                if shutdown.is_cancelled() {
                    break;
                }
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
                            log_attempt(result)?;
                        }
                    }
                }
            }
        }

        while let Some(result) = running.join_next().await {
            log_attempt(result)?;
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

fn drain_finished(running: &mut JoinSet<Result<(), RunnerError>>) -> Result<(), RunnerError> {
    while let Some(result) = running.try_join_next() {
        log_attempt(result)?;
    }
    Ok(())
}

/// One attempt's lease loss or store error stays local to that attempt; only
/// a panicked or aborted task reaches runtime supervision.
fn log_attempt(
    result: Result<Result<(), RunnerError>, tokio::task::JoinError>,
) -> Result<(), RunnerError> {
    if let Err(error) = result? {
        tracing::warn!(error = %error, "job attempt could not record its outcome");
    }
    Ok(())
}

fn claim_or_log(
    result: Result<Result<Option<ClaimedJob>, crate::StoreError>, tokio::task::JoinError>,
) -> Option<ClaimedJob> {
    match result {
        Ok(Ok(job)) => job,
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "job claim failed");
            None
        }
        Err(error) => {
            tracing::warn!(error = %error, "job claim task failed");
            None
        }
    }
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
    let lease = JobLease::of(&job, config.worker_id.as_str());
    let cancellation = JobCancellation::new(lease.clone());
    let result = run_attempt(
        store.as_ref(),
        handler.as_ref(),
        &config,
        &job,
        &cancellation,
    )
    .await?;
    let outcome = if cancellation.was_completed_in_transaction() {
        SUCCESS
    } else {
        settle_attempt(store.as_ref(), &config, &job, &lease, result).await?
    };
    metrics::record(
        &job.job_type,
        handler.job_types(),
        outcome,
        started.elapsed(),
    );
    Ok(())
}

async fn run_attempt(
    store: &dyn JobStore,
    handler: &dyn JobHandler,
    config: &WorkerConfig,
    job: &ClaimedJob,
    cancellation: &JobCancellation,
) -> Result<AttemptResult, RunnerError> {
    let future = handler.handle(job, cancellation.clone());
    tokio::pin!(future);
    let timeout = tokio::time::sleep(config.job_timeout);
    tokio::pin!(timeout);
    let cancellation_poll = wait_for_cancellation(store, job.id, config.cancellation_poll_interval);
    tokio::pin!(cancellation_poll);

    Ok(tokio::select! {
        result = &mut future => AttemptResult::Handled(result),
        () = &mut timeout => {
            cancellation.cancel();
            AttemptResult::Handled(Err(JobError::retryable("job timed out")))
        }
        result = &mut cancellation_poll => {
            result?;
            cancellation.cancel();
            AttemptResult::Cancelled
        }
    })
}

async fn settle_attempt(
    store: &dyn JobStore,
    config: &WorkerConfig,
    job: &ClaimedJob,
    lease: &JobLease,
    result: AttemptResult,
) -> Result<&'static str, RunnerError> {
    match result {
        AttemptResult::Cancelled => {
            let cancelled = store.cancel(lease, Timestamp::now()).await?;
            ensure_transition(cancelled, job.id)?;
            Ok(FAILURE)
        }
        AttemptResult::Handled(Ok(())) => {
            let now = Timestamp::now();
            if store.complete(lease, now).await? {
                return Ok(SUCCESS);
            }
            cancel_after_lost_transition(store, lease, now).await
        }
        AttemptResult::Handled(Err(error)) => {
            let now = Timestamp::now();
            let delay = error.retry_after().map_or_else(
                || retry_delay(config, job.attempts),
                |after| after.min(config.retry_max),
            );
            let retry_at = add_duration(now, delay)?;
            let disposition = store
                .record_failure(
                    lease,
                    error.is_retryable(),
                    retry_at,
                    &error.to_string(),
                    now,
                )
                .await?;
            match disposition {
                Some(FailureDisposition::Retry) => Ok(RETRY),
                Some(FailureDisposition::Failed) => Ok(FAILURE),
                None => cancel_after_lost_transition(store, lease, now).await,
            }
        }
    }
}

/// A completion or failure write that matched no lease is a cancellation only
/// when one was requested; otherwise another worker owns the job.
async fn cancel_after_lost_transition(
    store: &dyn JobStore,
    lease: &JobLease,
    now: jiff::Timestamp,
) -> Result<&'static str, RunnerError> {
    if store.cancellation_requested(lease.job_id).await? && store.cancel(lease, now).await? {
        Ok(FAILURE)
    } else {
        Err(RunnerError::LeaseLost {
            job_id: lease.job_id,
        })
    }
}

async fn wait_for_cancellation(
    store: &dyn JobStore,
    job_id: Uuid,
    poll_interval: Duration,
) -> Result<(), crate::StoreError> {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        if store.cancellation_requested(job_id).await? {
            return Ok(());
        }
    }
}

fn retry_delay(config: &WorkerConfig, attempts: u32) -> Duration {
    let exponent = attempts.saturating_sub(1).min(31);
    config
        .retry_initial
        .saturating_mul(2_u32.saturating_pow(exponent))
        .min(config.retry_max)
}

fn add_duration(
    timestamp: jiff::Timestamp,
    duration: Duration,
) -> Result<jiff::Timestamp, RunnerError> {
    let duration = SignedDuration::try_from(duration)
        .map_err(|_| RunnerError::InvalidConfig("retry delay is out of range".to_owned()))?;
    timestamp
        .checked_add(duration)
        .map_err(|_| RunnerError::InvalidConfig("retry deadline is out of range".to_owned()))
}

fn ensure_transition(updated: bool, job_id: Uuid) -> Result<(), RunnerError> {
    if updated {
        Ok(())
    } else {
        Err(RunnerError::LeaseLost { job_id })
    }
}

#[cfg(test)]
mod tests;
