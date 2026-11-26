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
use jiff::Timestamp;
use tokio::{
    task::{JoinError, JoinHandle, JoinSet},
    time::Instant,
};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{ClaimedJob, JobError, JobStore, ReleaseReason, RunnerError, StoreError, metrics};

mod attempt;
mod backoff;
mod config;

use attempt::{AttemptContext, process_claimed_job};
use backoff::StoreBackoff;
pub use backoff::StoreHealth;
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
    worker_id: Arc<str>,
    completed_in_transaction: Arc<AtomicBool>,
}

impl JobCancellation {
    fn new(worker_id: &str, drain: &CancellationToken) -> Self {
        Self {
            token: drain.child_token(),
            worker_id: Arc::from(worker_id),
            completed_in_transaction: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns the lease owner required by `PostgresJobStore::complete_in_transaction`.
    pub fn worker_id(&self) -> &str {
        &self.worker_id
    }

    /// Reports that the handler committed its result and successful job transition together.
    ///
    /// Call this only after the transaction containing `complete_in_transaction`
    /// commits successfully, and return success immediately afterward. The runner
    /// then records success without attempting a second completion transition.
    pub fn mark_completed_in_transaction(&self) {
        self.completed_in_transaction.store(true, Ordering::Release);
    }

    /// Returns whether timeout, cancellation or worker shutdown has stopped this attempt.
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
    /// Delay between empty-queue polls, and the first delay after a store error.
    pub poll_interval: Duration,
    /// Duration of a claimed job lease. Running attempts renew it every third of this.
    pub lease_duration: Duration,
    /// Maximum execution time for one attempt.
    pub job_timeout: Duration,
    /// Frequency at which running attempts check durable cancellation.
    pub cancellation_poll_interval: Duration,
    /// Initial retry delay before exponential growth.
    pub retry_initial: Duration,
    /// Upper bound for exponential retry delay.
    pub retry_max: Duration,
    /// Deferrals per job that do not consume an attempt; later deferrals do.
    pub max_deferrals: u32,
    /// Interval between queue age and depth samples.
    pub queue_sample_interval: Duration,
    /// Time kept before the shutdown drain deadline to release attempts still running.
    pub release_reserve: Duration,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            worker_id: format!("worker-{}", Uuid::now_v7()),
            queue: "default",
            concurrency: 5,
            poll_interval: Duration::from_millis(250),
            lease_duration: Duration::from_secs(60),
            job_timeout: Duration::from_secs(10 * 60),
            cancellation_poll_interval: Duration::from_millis(250),
            retry_initial: Duration::from_secs(1),
            retry_max: Duration::from_secs(5 * 60),
            max_deferrals: 100,
            queue_sample_interval: Duration::from_secs(15),
            release_reserve: Duration::from_secs(5),
        }
    }
}

/// Concurrent durable worker loop.
#[derive(Clone)]
pub struct WorkerRunner {
    store: Arc<dyn JobStore>,
    handler: Arc<dyn JobHandler>,
    config: WorkerConfig,
    health: StoreHealth,
}

enum ClaimError {
    Store(StoreError),
    Fatal(RunnerError),
}

impl WorkerRunner {
    /// Validates configuration, registers zero-valued worker metrics, and creates a runner.
    ///
    /// # Errors
    ///
    /// [`RunnerError::InvalidConfig`] for an empty worker id or queue, zero concurrency or
    /// duration, `retry_initial` above `retry_max`, a lease shorter than one second, or empty
    /// or duplicate job types.
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
            health: StoreHealth::new(),
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

    /// Returns the store health signal [`Self::run`] updates.
    ///
    /// Take it before calling `run`, which consumes the runner.
    #[must_use]
    pub fn store_health(&self) -> StoreHealth {
        self.health.clone()
    }

    /// Claims and executes jobs until process shutdown begins.
    ///
    /// Every claim passes the handler's [`JobHandler::job_types`], so the runner
    /// never claims a job type its handler does not declare.
    ///
    /// Store errors inside the loop are logged, flip [`Self::store_health`] to
    /// unhealthy and are retried with jittered backoff capped at 30 s. A lost
    /// lease is a per-job warning. Running attempts are never aborted because
    /// of a store error.
    ///
    /// After shutdown begins the runner stops claiming, releases a claim that
    /// commits after that point, and cancels every attempt's
    /// [`JobCancellation`]. It waits for attempts until
    /// [`WorkerConfig::release_reserve`] before the [`ShutdownToken`] deadline,
    /// then drops the handlers still running and releases their jobs to pending
    /// without consuming an attempt. A retryable failure returned during drain
    /// is released the same way.
    ///
    /// # Errors
    ///
    /// [`RunnerError::Task`] when a claim or attempt task panics or is aborted, and
    /// [`RunnerError::InvalidConfig`] when a retry deadline overflows the timestamp range.
    pub async fn run(self, shutdown: ShutdownToken) -> Result<(), RunnerError> {
        let context = Arc::new(AttemptContext {
            store: Arc::clone(&self.store),
            handler: Arc::clone(&self.handler),
            config: self.config.clone(),
            drain: CancellationToken::new(),
            release_now: CancellationToken::new(),
        });
        let mut running = JoinSet::new();
        let mut backoff = StoreBackoff::new(self.config.poll_interval);
        let mut next_sample = Instant::now();
        while !shutdown.is_cancelled() {
            drain_finished(&mut running)?;
            let step = match self.sample_queue_when_due(&mut next_sample).await {
                Ok(()) => {
                    self.claim_available(&mut running, &context, &shutdown)
                        .await
                }
                Err(error) => Err(ClaimError::Store(error)),
            };
            match step {
                Ok(()) => {
                    self.health.set(true);
                    backoff.reset();
                    wait_for_work(&mut running, &shutdown, self.config.poll_interval).await?;
                }
                Err(ClaimError::Store(error)) => {
                    self.health.set(false);
                    let delay = backoff.next_delay();
                    tracing::error!(queue = self.config.queue, %error, retry_in_ms = delay.as_millis(), "worker store call failed");
                    wait_for_work(&mut running, &shutdown, delay).await?;
                }
                Err(ClaimError::Fatal(error)) => return Err(error),
            }
        }
        drain_attempts(running, &context, &shutdown).await
    }

    async fn sample_queue_when_due(&self, next_sample: &mut Instant) -> Result<(), StoreError> {
        if Instant::now() < *next_sample {
            return Ok(());
        }
        let job_types = self.handler.job_types();
        let age = self
            .store
            .oldest_pending_age(job_types, Timestamp::now())
            .await?;
        let depth = self.store.queue_depth(job_types).await?;
        metrics::set_queue_age(self.config.queue, age);
        metrics::set_queue_depth(self.config.queue, depth);
        *next_sample = Instant::now() + self.config.queue_sample_interval;
        Ok(())
    }

    async fn claim_available(
        &self,
        running: &mut JoinSet<Result<(), RunnerError>>,
        context: &Arc<AttemptContext>,
        shutdown: &ShutdownToken,
    ) -> Result<(), ClaimError> {
        while running.len() < self.config.concurrency && !shutdown.is_cancelled() {
            let mut claim = self.spawn_claim();
            let claimed = tokio::select! {
                biased;
                result = &mut claim => claim_result(result)?,
                () = shutdown.cancelled() => {
                    release_late_claim(context, claim, release_deadline(shutdown, &self.config)).await;
                    return Ok(());
                }
            };
            let Some(job) = claimed else {
                return Ok(());
            };
            if shutdown.is_cancelled() {
                release_claimed(context, &job).await;
                return Ok(());
            }
            let context = Arc::clone(context);
            running.spawn(process_claimed_job(context, job));
        }
        Ok(())
    }

    fn spawn_claim(&self) -> JoinHandle<Result<Option<ClaimedJob>, StoreError>> {
        let store = Arc::clone(&self.store);
        let worker_id = self.config.worker_id.clone();
        let job_types = self.handler.job_types();
        let lease_duration = self.config.lease_duration;
        // Jobs enqueued during this loop must not be claimed before their creation time.
        tokio::spawn(async move {
            store
                .claim(&worker_id, job_types, Timestamp::now(), lease_duration)
                .await
        })
    }
}

fn claim_result(
    result: Result<Result<Option<ClaimedJob>, StoreError>, JoinError>,
) -> Result<Option<ClaimedJob>, ClaimError> {
    result
        .map_err(|error| ClaimError::Fatal(RunnerError::Task(error)))?
        .map_err(ClaimError::Store)
}

/// A claim that commits after shutdown began goes straight back to pending.
async fn release_late_claim(
    context: &AttemptContext,
    claim: JoinHandle<Result<Option<ClaimedJob>, StoreError>>,
    release_at: Instant,
) {
    let Ok(claimed) = tokio::time::timeout_at(release_at, claim).await else {
        tracing::warn!(
            "claim in flight at shutdown outlived the drain budget; its lease expiry recovers the job"
        );
        return;
    };
    match claimed {
        Ok(Ok(Some(job))) => release_claimed(context, &job).await,
        Ok(Ok(None)) => {}
        Ok(Err(error)) => tracing::warn!(%error, "claim in flight at shutdown failed"),
        Err(error) => tracing::error!(%error, "claim task in flight at shutdown failed"),
    }
}

async fn release_claimed(context: &AttemptContext, job: &ClaimedJob) {
    let now = Timestamp::now();
    match context
        .store
        .release(
            job.id,
            &context.config.worker_id,
            now,
            ReleaseReason::Shutdown,
            now,
        )
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => tracing::warn!(job_id = %job.id, "job claimed at shutdown was no longer owned"),
        Err(error) => {
            tracing::error!(job_id = %job.id, %error, "job claimed at shutdown was not released; it is reclaimed after its lease");
        }
    }
}

async fn wait_for_work(
    running: &mut JoinSet<Result<(), RunnerError>>,
    shutdown: &ShutdownToken,
    delay: Duration,
) -> Result<(), RunnerError> {
    if running.is_empty() {
        tokio::select! {
            () = shutdown.cancelled() => {},
            () = tokio::time::sleep(delay) => {},
        }
        return Ok(());
    }
    tokio::select! {
        () = shutdown.cancelled() => {},
        () = tokio::time::sleep(delay) => {},
        result = running.join_next() => {
            if let Some(result) = result {
                result??;
            }
        }
    }
    Ok(())
}

fn drain_finished(running: &mut JoinSet<Result<(), RunnerError>>) -> Result<(), RunnerError> {
    while let Some(result) = running.try_join_next() {
        result??;
    }
    Ok(())
}

fn release_deadline(shutdown: &ShutdownToken, config: &WorkerConfig) -> Instant {
    shutdown
        .deadline()
        .unwrap_or_else(|| Instant::now() + shutdown.drain_timeout())
        .checked_sub(config.release_reserve)
        .unwrap_or_else(Instant::now)
}

async fn drain_attempts(
    mut running: JoinSet<Result<(), RunnerError>>,
    context: &AttemptContext,
    shutdown: &ShutdownToken,
) -> Result<(), RunnerError> {
    context.drain.cancel();
    let release_at = release_deadline(shutdown, &context.config);
    let mut first_error = None;
    loop {
        let next = tokio::select! {
            next = running.join_next() => next,
            () = tokio::time::sleep_until(release_at), if !context.release_now.is_cancelled() => {
                context.release_now.cancel();
                continue;
            }
        };
        let Some(result) = next else {
            break;
        };
        if let Err(error) = result.map_err(RunnerError::from).and_then(|result| result) {
            first_error.get_or_insert(error);
        }
    }
    first_error.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests;
