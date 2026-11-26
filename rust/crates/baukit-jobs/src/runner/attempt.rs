use std::{sync::Arc, time::Duration};

use baukit_resilience::Backoff;
use jiff::{SignedDuration, Timestamp};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::{
    ClaimedJob, FailureDisposition, JobError, JobLease, JobStore, RecurringJob, ReleaseReason,
    RunnerError, StoreError,
    metrics::{self, CANCELLED, FAILURE, RETRY, SUCCESS},
};

use super::{JobCancellation, JobHandler, SharedJitter, WorkerConfig};

const HEARTBEATS_PER_LEASE: u32 = 3;

/// Shared state every attempt of one runner needs.
pub(super) struct AttemptContext {
    pub(super) store: Arc<dyn JobStore>,
    pub(super) handler: Arc<dyn JobHandler>,
    pub(super) config: WorkerConfig,
    /// Cancelled when drain begins; parent of every attempt's cancellation.
    pub(super) drain: CancellationToken,
    /// Cancelled when the drain budget is spent; attempts still running release.
    pub(super) release_now: CancellationToken,
    pub(super) jitter: SharedJitter,
    /// Recurring job types whose next slot the runner enqueues after a terminal failure.
    pub(super) recurring: Vec<RecurringJob>,
}

enum AttemptResult {
    Handled(Result<(), JobError>),
    TimedOut,
    Cancelled,
    LeaseLost,
    Interrupted,
}

enum Unsettled {
    LeaseLost,
    Store(StoreError),
    Fatal(RunnerError),
}

impl From<StoreError> for Unsettled {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

pub(super) async fn process_claimed_job(
    context: Arc<AttemptContext>,
    job: ClaimedJob,
) -> Result<(), RunnerError> {
    let started = Instant::now();
    let cancellation = JobCancellation::new(attempt_lease(context.as_ref(), &job), &context.drain);
    let result = run_attempt(&context, &job, &cancellation).await;
    let settled = if cancellation.was_completed_in_transaction() {
        Ok(SUCCESS)
    } else {
        settle_attempt(&context, &job, result).await
    };
    match settled {
        Ok(outcome) => {
            metrics::record(
                &job.job_type,
                context.handler.job_types(),
                outcome,
                started.elapsed(),
            );
            Ok(())
        }
        Err(Unsettled::LeaseLost) => {
            tracing::warn!(job_id = %job.id, job_kind = %job.job_type, "worker lost the job lease before recording its outcome");
            metrics::record_lease_expired(context.config.queue);
            Ok(())
        }
        Err(Unsettled::Store(error)) => {
            tracing::error!(job_id = %job.id, job_kind = %job.job_type, %error, "job outcome was not recorded; the lease expires and the job is reclaimed");
            Ok(())
        }
        Err(Unsettled::Fatal(error)) => Err(error),
    }
}

async fn run_attempt(
    context: &AttemptContext,
    job: &ClaimedJob,
    cancellation: &JobCancellation,
) -> AttemptResult {
    let config = &context.config;
    let store = context.store.as_ref();
    let future = context.handler.handle(job, cancellation.clone());
    tokio::pin!(future);
    let timeout = tokio::time::sleep(config.job_timeout);
    tokio::pin!(timeout);
    let cancellation_poll = wait_for_cancellation(store, job.id, config.cancellation_poll_interval);
    tokio::pin!(cancellation_poll);
    let lease = renew_lease_until_lost(store, cancellation.lease(), config.lease_duration);
    tokio::pin!(lease);

    let result = tokio::select! {
        result = &mut future => return AttemptResult::Handled(result),
        () = &mut timeout => AttemptResult::TimedOut,
        () = &mut cancellation_poll => AttemptResult::Cancelled,
        () = &mut lease => AttemptResult::LeaseLost,
        () = context.release_now.cancelled() => AttemptResult::Interrupted,
    };
    cancellation.cancel();
    result
}

async fn settle_attempt(
    context: &AttemptContext,
    job: &ClaimedJob,
    result: AttemptResult,
) -> Result<&'static str, Unsettled> {
    let store = context.store.as_ref();
    let lease = attempt_lease(context, job);
    match result {
        AttemptResult::Cancelled => {
            if store.cancel(&lease, Timestamp::now()).await? {
                Ok(CANCELLED)
            } else {
                Err(Unsettled::LeaseLost)
            }
        }
        AttemptResult::LeaseLost => Err(Unsettled::LeaseLost),
        AttemptResult::Interrupted => release_for_shutdown(context, job).await,
        AttemptResult::TimedOut => {
            record_failure(context, job, &JobError::retryable("job timed out")).await
        }
        AttemptResult::Handled(Ok(())) => {
            let now = Timestamp::now();
            if store.complete(&lease, now).await? {
                return Ok(SUCCESS);
            }
            cancel_after_lost_transition(context, job, now).await
        }
        AttemptResult::Handled(Err(error)) => {
            if let Some(delay) = error.deferral() {
                return defer(context, job, delay).await;
            }
            if error.is_retryable() && context.drain.is_cancelled() {
                return release_for_shutdown(context, job).await;
            }
            record_failure(context, job, &error).await
        }
    }
}

async fn release_for_shutdown(
    context: &AttemptContext,
    job: &ClaimedJob,
) -> Result<&'static str, Unsettled> {
    let now = Timestamp::now();
    let released = context
        .store
        .release(
            &attempt_lease(context, job),
            now,
            ReleaseReason::Shutdown,
            now,
        )
        .await?;
    disposition_outcome(context, job, released, now).await
}

async fn defer(
    context: &AttemptContext,
    job: &ClaimedJob,
    delay: Duration,
) -> Result<&'static str, Unsettled> {
    metrics::record_deferral(&job.job_type, context.handler.job_types());
    let reason = if job.deferrals >= context.config.max_deferrals {
        ReleaseReason::DeferredPastCap
    } else {
        ReleaseReason::Deferred
    };
    let now = Timestamp::now();
    let run_after = add_duration(now, delay).map_err(Unsettled::Fatal)?;
    let released = context
        .store
        .release(&attempt_lease(context, job), run_after, reason, now)
        .await?;
    disposition_outcome(context, job, released, now).await
}

async fn record_failure(
    context: &AttemptContext,
    job: &ClaimedJob,
    error: &JobError,
) -> Result<&'static str, Unsettled> {
    let now = Timestamp::now();
    let delay = error.retry_after().map_or_else(
        || {
            context
                .jitter
                .delay(&retry_backoff(&context.config), retry_index(job.attempts))
        },
        |after| after.min(context.config.retry_max),
    );
    let retry_at = add_duration(now, delay).map_err(Unsettled::Fatal)?;
    let disposition = context
        .store
        .record_failure(
            &attempt_lease(context, job),
            error.is_retryable(),
            retry_at,
            &error.to_string(),
            now,
        )
        .await?;
    disposition_outcome(context, job, disposition, now).await
}

async fn disposition_outcome(
    context: &AttemptContext,
    job: &ClaimedJob,
    disposition: Option<FailureDisposition>,
    now: Timestamp,
) -> Result<&'static str, Unsettled> {
    match disposition {
        Some(FailureDisposition::Retry) => Ok(RETRY),
        Some(FailureDisposition::Failed) => {
            schedule_next_slot(context, job).await;
            Ok(FAILURE)
        }
        None => cancel_after_lost_transition(context, job, now).await,
    }
}

/// A transition that matched no lease is a cancellation only when one was
/// requested; otherwise another worker owns the job.
async fn cancel_after_lost_transition(
    context: &AttemptContext,
    job: &ClaimedJob,
    now: Timestamp,
) -> Result<&'static str, Unsettled> {
    let store = context.store.as_ref();
    if store.cancellation_requested(job.id).await?
        && store.cancel(&attempt_lease(context, job), now).await?
    {
        Ok(CANCELLED)
    } else {
        Err(Unsettled::LeaseLost)
    }
}

async fn wait_for_cancellation(store: &dyn JobStore, job_id: Uuid, poll_interval: Duration) {
    let mut interval = tokio::time::interval(poll_interval);
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);
    interval.tick().await;
    loop {
        interval.tick().await;
        match store.cancellation_requested(job_id).await {
            Ok(true) => return,
            Ok(false) => {}
            Err(error) => tracing::warn!(%job_id, %error, "cancellation poll failed"),
        }
    }
}

async fn renew_lease_until_lost(store: &dyn JobStore, lease: &JobLease, lease_duration: Duration) {
    let job_id = lease.job_id;
    let mut interval = tokio::time::interval(lease_duration / HEARTBEATS_PER_LEASE);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval.tick().await;
    loop {
        interval.tick().await;
        match store
            .heartbeat(lease, lease_duration, Timestamp::now())
            .await
        {
            Ok(true) => {}
            Ok(false) => return,
            Err(error) => tracing::warn!(%job_id, %error, "lease heartbeat failed"),
        }
    }
}

fn attempt_lease(context: &AttemptContext, job: &ClaimedJob) -> JobLease {
    JobLease::of(job, context.config.worker_id.as_str())
}

/// Keeps a recurring chain alive when one slot fails for good. The slot's
/// idempotency key makes a concurrent enqueue of the same slot harmless.
async fn schedule_next_slot(context: &AttemptContext, job: &ClaimedJob) {
    let Some(recurring) = context
        .recurring
        .iter()
        .find(|recurring| recurring.job_type() == job.job_type)
    else {
        return;
    };
    let next = match recurring.next_job(job, Timestamp::now()) {
        Ok(next) => next,
        Err(error) => {
            tracing::error!(job_id = %job.id, job_kind = %job.job_type, %error, "failed recurring slot has no next slot");
            return;
        }
    };
    if let Err(error) = context.store.enqueue(next).await {
        tracing::error!(job_id = %job.id, job_kind = %job.job_type, %error, "next recurring slot was not enqueued; enqueue_initial recovers it at the next start");
    }
}

pub(super) const fn retry_backoff(config: &WorkerConfig) -> Backoff {
    Backoff::new(config.retry_initial, config.retry_max)
}

/// Zero-based retry index for a job that has used `attempts` claims.
pub(super) const fn retry_index(attempts: u32) -> u32 {
    attempts.saturating_sub(1)
}

#[cfg(test)]
pub(super) fn retry_delay(config: &WorkerConfig, attempts: u32, fraction: f64) -> Duration {
    retry_backoff(config).delay(retry_index(attempts), fraction)
}

fn add_duration(timestamp: Timestamp, duration: Duration) -> Result<Timestamp, RunnerError> {
    let duration = SignedDuration::try_from(duration)
        .map_err(|_| RunnerError::InvalidConfig("retry delay is out of range".to_owned()))?;
    timestamp
        .checked_add(duration)
        .map_err(|_| RunnerError::InvalidConfig("retry deadline is out of range".to_owned()))
}
