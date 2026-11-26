use jiff_sqlx::{Timestamp as SqlTimestamp, ToSqlx};
use std::{future::Future, num::NonZeroU32, pin::Pin, time::Duration};

use jiff::Timestamp;
use sqlx::{PgConnection, PgPool, Postgres, Row as _, Transaction};
use uuid::Uuid;

use crate::{
    CancellationOutcome, ClaimedJob, EnqueueOutcome, FailureDisposition, Job, JobStatus, NewJob,
    QueueDepth, ReleaseReason, StoreError, TerminalJobCleanupOutcome, TerminalJobCutoffs,
};

mod claim;
mod enqueue;
mod lease;
mod listener;
mod queue;
mod requeue;
mod validation;

use claim::{ClaimRequest, claim_on};
use enqueue::{enqueue_deduplicated_on, enqueue_on};
pub use listener::{JOB_ENQUEUED_CHANNEL, spawn_enqueue_listener};
pub use requeue::MAX_REQUEUE_BATCH_SIZE;
use requeue::{requeue_failed_on, validate_requeue};

/// The claim's candidate query, exposed so tests can `EXPLAIN` it.
#[doc(hidden)]
pub const CLAIM_CANDIDATE_SQL: &str = claim::CANDIDATE_SQL;
use lease::{heartbeat_on, lease_deadline, release_on};
use queue::{oldest_ready_age_on, queue_depth_on};

use validation::{
    bound_error, parse_failure_reason, parse_status, status_name, validate_deduplicated_job,
    validate_job_types, validate_new_job, validate_text, validate_worker_id,
};

const MAX_JOB_TYPE_LENGTH: usize = 200;
const MAX_WORKER_ID_LENGTH: usize = 300;
const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 500;
const MAX_DEDUPE_KEY_LENGTH: usize = 500;
const MAX_CONCURRENCY_KEY_LENGTH: usize = 500;
const MAX_ERROR_LENGTH: usize = 10_000;

#[derive(Clone, Copy)]
enum Deduplication {
    Active,
    Pending,
}

macro_rules! owned_lease_predicate {
    () => {
        "status = 'running' AND locked_by = $2 AND locked_until > $3 AND ($4 OR cancel_requested_at IS NULL)"
    };
}

macro_rules! cancellation_assignments {
    () => {
        "status = CASE WHEN status = 'pending' THEN 'cancelled' ELSE status END, locked_by = CASE WHEN status = 'pending' THEN NULL ELSE locked_by END, locked_until = CASE WHEN status = 'pending' THEN NULL ELSE locked_until END, cancel_requested_at = CASE WHEN status = 'running' THEN $1 ELSE NULL END, updated_at = $1"
    };
}

/// Largest terminal-job cleanup batch accepted by [`PostgresJobStore`].
pub const MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE: u32 = 10_000;

/// A boxed future returned by [`JobStore`] ports.
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Durable job lifecycle operations used by the worker runner.
///
/// Implementations must make claim and finish transitions conditional on lease
/// ownership. The PostgreSQL adapter uses `FOR UPDATE SKIP LOCKED` and performs
/// attempt increment in the same transaction as claim.
pub trait JobStore: Send + Sync + 'static {
    /// Enqueues a job, returning the existing row for a duplicate idempotency key.
    fn enqueue(&self, job: NewJob) -> StoreFuture<'_, Result<EnqueueOutcome, StoreError>>;

    /// Claims the oldest ready job whose type is in `job_types` and increments
    /// its attempt count atomically.
    ///
    /// Rows of other types stay untouched, so a type that no worker handles
    /// remains pending. An empty `job_types` set is invalid input because it
    /// can never claim a job. Concurrency keys are not limited; see
    /// [`Self::claim_with_key_limit`].
    fn claim<'a>(
        &'a self,
        worker_id: &'a str,
        job_types: &'a [&'a str],
        now: Timestamp,
        lease_for: Duration,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>> {
        self.claim_with_key_limit(worker_id, job_types, now, lease_for, None)
    }

    /// Claims like [`Self::claim`], skipping ready rows whose
    /// [`NewJob::concurrency_key`] already has `max_per_key` running jobs with
    /// a live lease. `None` claims regardless of keys.
    fn claim_with_key_limit<'a>(
        &'a self,
        worker_id: &'a str,
        job_types: &'a [&'a str],
        now: Timestamp,
        lease_for: Duration,
        max_per_key: Option<NonZeroU32>,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>>;

    /// Completes a currently owned job atomically.
    fn complete<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>>;

    /// Records a failed attempt, retrying only when permitted and attempts remain.
    fn record_failure<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        retryable: bool,
        retry_at: Timestamp,
        error: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>>;

    /// Returns whether cancellation was requested for a running job.
    fn cancellation_requested(&self, job_id: Uuid) -> StoreFuture<'_, Result<bool, StoreError>>;

    /// Requests cancellation, immediately cancelling pending jobs.
    fn request_cancellation(
        &self,
        job_id: Uuid,
        now: Timestamp,
    ) -> StoreFuture<'_, Result<bool, StoreError>>;

    /// Marks a currently owned job cancelled.
    fn cancel<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>>;

    /// Extends the lease of a job this worker still owns to `now + lease_for`.
    ///
    /// Returns false when the job is no longer running under this worker's
    /// unexpired lease.
    fn heartbeat<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        lease_for: Duration,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>>;

    /// Returns an owned running job to pending at `run_after`.
    ///
    /// `reason` decides whether the attempt is given back and whether
    /// `deferrals` grows; see [`ReleaseReason`]. A job with
    /// a cancellation request is not released. Returns `None` when this worker
    /// no longer owns the job.
    fn release<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        run_after: Timestamp,
        reason: ReleaseReason,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>>;

    /// Returns how long the oldest claimable pending job of `job_types` has
    /// been ready, or zero when none is ready.
    ///
    /// Rows scheduled after `now` do not count.
    fn oldest_pending_age<'a>(
        &'a self,
        job_types: &'a [&'a str],
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Duration, StoreError>>;

    /// Counts pending, running and failed rows of `job_types`.
    fn queue_depth<'a>(
        &'a self,
        job_types: &'a [&'a str],
    ) -> StoreFuture<'a, Result<QueueDepth, StoreError>>;

    /// Probes the same locking query shape used to claim work without claiming it.
    fn ready(&self) -> StoreFuture<'_, Result<(), StoreError>>;
}

/// PostgreSQL [`JobStore`] backed by a product-owned `job_outbox` table.
#[derive(Clone, Debug)]
pub struct PostgresJobStore {
    pool: PgPool,
    retained_kinds: Vec<String>,
    retained_failed_kinds: Vec<String>,
}

impl PostgresJobStore {
    /// Creates a store using an existing product pool.
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            retained_kinds: Vec::new(),
            retained_failed_kinds: Vec::new(),
        }
    }

    /// Retains jobs of these kinds in every terminal status for separate cleanup.
    #[must_use]
    pub fn retain_kinds(mut self, kinds: &[&str]) -> Self {
        self.retained_kinds = kinds.iter().map(|kind| (*kind).to_owned()).collect();
        self
    }

    /// Retains failed jobs of these kinds for inspection and repair.
    ///
    /// Successful and cancelled jobs of these kinds still follow their cutoffs
    /// unless also retained by [`Self::retain_kinds`].
    #[must_use]
    pub fn retain_failed_kinds(mut self, kinds: &[&str]) -> Self {
        self.retained_failed_kinds = kinds.iter().map(|kind| (*kind).to_owned()).collect();
        self
    }

    /// Returns the underlying pool for product-level transaction composition.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Enqueues inside a caller-owned transaction.
    ///
    /// Use this with a domain mutation to implement the transactional outbox
    /// pattern. The transaction remains owned by the caller.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the job fails validation or `max_attempts` exceeds
    /// `i32::MAX`, [`StoreError::Database`] when the insert fails, and
    /// [`StoreError::InvalidData`] when the stored row does not decode.
    pub async fn enqueue_in_transaction(
        &self,
        transaction: &mut PgConnection,
        job: NewJob,
    ) -> Result<EnqueueOutcome, StoreError> {
        validate_new_job(&job)?;
        enqueue_on(transaction, job).await
    }

    /// Enqueues or returns the pending or running job with the same type and key.
    ///
    /// Succeeded, failed, and cancelled jobs allow a new row. A duplicate keeps
    /// the original payload, schedule, and attempt budget. `dedupe_key` must
    /// contain 1 to 500 characters and cannot be blank. The job must not have
    /// an idempotency key. Apply migration 0004 before using this method.
    ///
    /// The store commits its transaction even if the caller is cancelled.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the job or `dedupe_key` fails validation or
    /// `max_attempts` exceeds `i32::MAX`, [`StoreError::Database`] when the transaction fails,
    /// [`StoreError::InvalidData`] when the stored row does not decode, and
    /// [`StoreError::Task`] when the owned store task panics or is aborted.
    pub async fn enqueue_deduplicated(
        &self,
        job: NewJob,
        dedupe_key: &str,
    ) -> Result<EnqueueOutcome, StoreError> {
        self.enqueue_with_deduplication(job, dedupe_key, Deduplication::Active)
            .await
    }

    /// Enqueues or returns the pending job with the same type and key.
    ///
    /// Running jobs allow one pending follow-up. Claiming clears the pending
    /// key, so retries of an already claimed job do not occupy it. This mode
    /// uses a separate key column from [`Self::enqueue_deduplicated`]; the same
    /// type and key in different modes do not deduplicate each other.
    /// Key validation and transaction ownership follow that method's rules.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the job or `dedupe_key` fails validation or
    /// `max_attempts` exceeds `i32::MAX`, [`StoreError::Database`] when the transaction fails,
    /// [`StoreError::InvalidData`] when the stored row does not decode, and
    /// [`StoreError::Task`] when the owned store task panics or is aborted.
    pub async fn enqueue_pending_deduplicated(
        &self,
        job: NewJob,
        dedupe_key: &str,
    ) -> Result<EnqueueOutcome, StoreError> {
        self.enqueue_with_deduplication(job, dedupe_key, Deduplication::Pending)
            .await
    }

    async fn enqueue_with_deduplication(
        &self,
        job: NewJob,
        dedupe_key: &str,
        deduplication: Deduplication,
    ) -> Result<EnqueueOutcome, StoreError> {
        validate_deduplicated_job(&job, dedupe_key)?;
        let pool = self.pool.clone();
        let dedupe_key = dedupe_key.to_owned();
        owned_operation(async move {
            let mut transaction = pool.begin().await.map_err(StoreError::database)?;
            let outcome =
                enqueue_deduplicated_on(&mut transaction, job, &dedupe_key, deduplication).await?;
            transaction.commit().await.map_err(StoreError::database)?;
            Ok(outcome)
        })
        .await
    }

    /// Enqueues with active-job deduplication in a caller-owned transaction.
    ///
    /// Uses the same rules as [`Self::enqueue_deduplicated`]. A returned
    /// existing row stays locked until the caller commits or rolls back, so
    /// keep the transaction short. The caller owns transaction completion.
    /// At repeatable read or serializable isolation, retry the whole transaction
    /// if PostgreSQL reports a serialization failure.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the job or `dedupe_key` fails validation or
    /// `max_attempts` exceeds `i32::MAX`, [`StoreError::Database`] when a statement fails,
    /// including serialization failures, and [`StoreError::InvalidData`] when the stored row
    /// does not decode.
    pub async fn enqueue_deduplicated_in_transaction(
        &self,
        transaction: &mut PgConnection,
        job: NewJob,
        dedupe_key: &str,
    ) -> Result<EnqueueOutcome, StoreError> {
        validate_deduplicated_job(&job, dedupe_key)?;
        enqueue_deduplicated_on(transaction, job, dedupe_key, Deduplication::Active).await
    }

    /// Enqueues with pending-only deduplication in a caller-owned transaction.
    ///
    /// Uses [`Self::enqueue_pending_deduplicated`] rules and holds an existing
    /// pending row locked until commit or rollback. Keep the transaction short.
    /// At repeatable read or serializable isolation, retry the whole transaction
    /// if PostgreSQL reports a serialization failure.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the job or `dedupe_key` fails validation or
    /// `max_attempts` exceeds `i32::MAX`, [`StoreError::Database`] when a statement fails,
    /// including serialization failures, and [`StoreError::InvalidData`] when the stored row
    /// does not decode.
    pub async fn enqueue_pending_deduplicated_in_transaction(
        &self,
        transaction: &mut PgConnection,
        job: NewJob,
        dedupe_key: &str,
    ) -> Result<EnqueueOutcome, StoreError> {
        validate_deduplicated_job(&job, dedupe_key)?;
        enqueue_deduplicated_on(transaction, job, dedupe_key, Deduplication::Pending).await
    }

    /// Marks a job successful inside a caller-owned transaction.
    ///
    /// Product writes performed earlier in the transaction and this lease-owned
    /// transition commit or roll back together.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] for an invalid `worker_id` and [`StoreError::Database`]
    /// when the update fails.
    pub async fn complete_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job_id: Uuid,
        worker_id: &str,
        now: Timestamp,
    ) -> Result<bool, StoreError> {
        validate_worker_id(worker_id)?;
        finish_on(
            transaction,
            job_id,
            worker_id,
            JobStatus::Succeeded,
            None,
            now,
        )
        .await
    }

    /// Checks lease ownership without completing the job, locking it until commit.
    ///
    /// Returns true only for a running job owned by `worker_id`, with a lease
    /// strictly later than `now` and no cancellation request. `SELECT FOR UPDATE`
    /// holds the row lock until the caller commits or rolls back. Concurrent
    /// cancellation or reclaim waits for that lock; claim's `SKIP LOCKED`
    /// selection skips the row. Time can still pass beyond the lease deadline,
    /// so completion must check the lease again using a fresh timestamp.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] for an invalid `worker_id` and [`StoreError::Database`]
    /// when the query fails.
    pub async fn check_lease_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job_id: Uuid,
        worker_id: &str,
        now: Timestamp,
    ) -> Result<bool, StoreError> {
        validate_worker_id(worker_id)?;
        sqlx::query_scalar::<_, Uuid>(concat!(
            "SELECT id FROM job_outbox WHERE id = $1 AND ",
            owned_lease_predicate!(),
            " FOR UPDATE"
        ))
        .bind(job_id)
        .bind(worker_id)
        .bind(now.to_sqlx())
        .bind(false)
        .fetch_optional(&mut **transaction)
        .await
        .map(|row| row.is_some())
        .map_err(StoreError::database)
    }

    /// Cancels jobs with an exact idempotency key inside the caller's transaction.
    ///
    /// Only `job_type` matches are changed. Pending jobs become cancelled with
    /// locks cleared. Running jobs retain their lease and receive a cancellation
    /// request for the runner to finish. Terminal jobs stay untouched.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] for a blank or overlong `job_type` or `key`,
    /// [`StoreError::Database`] when the update fails, and [`StoreError::InvalidData`] for an
    /// unknown returned status.
    pub async fn request_cancellation_by_key_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job_type: &str,
        key: &str,
        now: Timestamp,
    ) -> Result<CancellationOutcome, StoreError> {
        validate_text("job_type", job_type, MAX_JOB_TYPE_LENGTH)?;
        validate_text("idempotency_key", key, MAX_IDEMPOTENCY_KEY_LENGTH)?;
        request_cancellation_by_key_on(transaction, job_type, key, now).await
    }

    /// Cancels jobs with a literal key prefix inside the caller's transaction.
    ///
    /// The prefix must contain 1 to 500 characters and cannot be blank. `%` and
    /// `_` match literally. Only jobs of `job_type` are changed, using the same
    /// lifecycle as [`Self::request_cancellation_by_key_in_transaction`].
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] for a blank or overlong `job_type` or `prefix`,
    /// [`StoreError::Database`] when the update fails, and [`StoreError::InvalidData`] for an
    /// unknown returned status.
    pub async fn request_cancellation_by_key_prefix_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job_type: &str,
        prefix: &str,
        now: Timestamp,
    ) -> Result<CancellationOutcome, StoreError> {
        validate_text("job_type", job_type, MAX_JOB_TYPE_LENGTH)?;
        validate_text("idempotency_key_prefix", prefix, MAX_IDEMPOTENCY_KEY_LENGTH)?;
        request_cancellation_by_key_prefix_on(transaction, job_type, prefix, now).await
    }

    /// Returns failed jobs of `job_type` among `ids` to pending, ready now.
    ///
    /// Requeued rows restart with zero attempts and deferrals and no failure
    /// reason or last error. Ids of other types or statuses are ignored. A
    /// failed row whose active dedupe key a pending or running row holds stays
    /// failed, and of several failed rows sharing one key only the newest is
    /// requeued. Returns the requeued ids and notifies
    /// [`JOB_ENQUEUED_CHANNEL`] when there is at least one.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] for an invalid `job_type` or when `ids` is
    /// empty or longer than [`MAX_REQUEUE_BATCH_SIZE`], and
    /// [`StoreError::Database`] when the transaction fails.
    pub async fn requeue_failed(
        &self,
        job_type: &str,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, StoreError> {
        validate_requeue(job_type, ids)?;
        let mut transaction = self.pool.begin().await.map_err(StoreError::database)?;
        let requeued = requeue_failed_on(&mut transaction, job_type, ids, Timestamp::now()).await?;
        transaction.commit().await.map_err(StoreError::database)?;
        Ok(requeued)
    }

    /// Deletes at most one batch of terminal jobs.
    ///
    /// A row is eligible when its status is `succeeded`, `cancelled`, or
    /// `failed` and its `updated_at` is earlier than that status's cutoff.
    /// Kinds configured with [`Self::retain_kinds`] are never eligible. Kinds
    /// configured with [`Self::retain_failed_kinds`] are ineligible when failed.
    /// Pending and running jobs are never eligible, including running jobs
    /// whose lease has expired. Concurrently locked rows are skipped.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when `batch_size` is zero or above the maximum,
    /// [`StoreError::Database`] when the delete fails, and [`StoreError::InvalidData`] for an
    /// unknown returned status.
    pub async fn cleanup_terminal_jobs(
        &self,
        cutoffs: TerminalJobCutoffs,
        batch_size: u32,
    ) -> Result<TerminalJobCleanupOutcome, StoreError> {
        if batch_size == 0 || batch_size > MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE {
            return Err(StoreError::InvalidInput(format!(
                "terminal-job cleanup batch size must be between 1 and {MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE}"
            )));
        }

        let deleted_statuses: Vec<String> = sqlx::query_scalar(
            r#"WITH candidates AS (
                SELECT id FROM job_outbox
                WHERE NOT (job_type = ANY($6)) AND (
                    (status = 'succeeded' AND updated_at < $1)
                    OR (status = 'cancelled' AND updated_at < $2)
                    OR (status = 'failed' AND updated_at < $3 AND NOT (job_type = ANY($5)))
                )
                ORDER BY updated_at, id FOR UPDATE SKIP LOCKED LIMIT $4
            )
            DELETE FROM job_outbox AS job USING candidates
            WHERE job.id = candidates.id RETURNING job.status"#,
        )
        .bind(cutoffs.succeeded_before.to_sqlx())
        .bind(cutoffs.cancelled_before.to_sqlx())
        .bind(cutoffs.failed_before.to_sqlx())
        .bind(i64::from(batch_size))
        .bind(&self.retained_failed_kinds)
        .bind(&self.retained_kinds)
        .fetch_all(&self.pool)
        .await
        .map_err(StoreError::database)?;

        let mut outcome = TerminalJobCleanupOutcome::default();
        for status in deleted_statuses {
            match status.as_str() {
                "succeeded" => outcome.succeeded += 1,
                "cancelled" => outcome.cancelled += 1,
                "failed" => outcome.failed += 1,
                value => {
                    return Err(StoreError::InvalidData(format!(
                        "terminal-job cleanup returned status `{value}`"
                    )));
                }
            }
        }
        Ok(outcome)
    }
}

impl JobStore for PostgresJobStore {
    fn enqueue(&self, job: NewJob) -> StoreFuture<'_, Result<EnqueueOutcome, StoreError>> {
        let pool = self.pool.clone();
        Box::pin(owned_operation(async move {
            validate_new_job(&job)?;
            let mut transaction = pool.begin().await.map_err(StoreError::database)?;
            let outcome = enqueue_on(&mut transaction, job).await?;
            transaction.commit().await.map_err(StoreError::database)?;
            Ok(outcome)
        }))
    }

    fn claim_with_key_limit<'a>(
        &'a self,
        worker_id: &'a str,
        job_types: &'a [&'a str],
        now: Timestamp,
        lease_for: Duration,
        max_per_key: Option<NonZeroU32>,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>> {
        let pool = self.pool.clone();
        let worker_id = worker_id.to_owned();
        let job_types = owned_job_types(job_types);
        Box::pin(owned_operation(async move {
            validate_worker_id(&worker_id)?;
            validate_job_types(&job_types.iter().map(String::as_str).collect::<Vec<_>>())?;
            let request = ClaimRequest {
                worker_id: &worker_id,
                job_types: &job_types,
                now,
                locked_until: lease_deadline(now, lease_for)?,
                max_per_key,
            };
            let mut transaction = pool.begin().await.map_err(StoreError::database)?;
            let claimed = claim_on(&mut transaction, &request).await?;
            transaction.commit().await.map_err(StoreError::database)?;
            Ok(claimed)
        }))
    }

    fn complete<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            let mut connection = self.pool.acquire().await.map_err(StoreError::database)?;
            finish_on(
                &mut connection,
                job_id,
                worker_id,
                JobStatus::Succeeded,
                None,
                now,
            )
            .await
        })
    }

    fn record_failure<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        retryable: bool,
        retry_at: Timestamp,
        error: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            let row = sqlx::query(
                "UPDATE job_outbox SET status = CASE WHEN $3 AND attempts < max_attempts THEN 'pending' ELSE 'failed' END, run_after = CASE WHEN $3 AND attempts < max_attempts THEN $4 ELSE run_after END, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = $5, failure_reason = CASE WHEN $3 AND attempts < max_attempts THEN NULL WHEN $3 THEN 'attempts_exhausted' ELSE 'permanent' END, updated_at = $6 WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $6 RETURNING status",
            )
            .bind(job_id)
            .bind(worker_id)
            .bind(retryable)
            .bind(retry_at.to_sqlx())
            .bind(bound_error(error))
            .bind(now.to_sqlx())
            .fetch_optional(&self.pool)
            .await
            .map_err(StoreError::database)?;
            row.map(|row| match row.try_get::<String, _>("status")?.as_str() {
                "pending" => Ok(FailureDisposition::Retry),
                "failed" => Ok(FailureDisposition::Failed),
                value => Err(StoreError::InvalidData(format!(
                    "failure transition returned status `{value}`"
                ))),
            })
            .transpose()
        })
    }

    fn cancellation_requested(&self, job_id: Uuid) -> StoreFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            sqlx::query_scalar::<_, bool>(
                "SELECT cancel_requested_at IS NOT NULL FROM job_outbox WHERE id = $1",
            )
            .bind(job_id)
            .fetch_optional(&self.pool)
            .await
            .map(|value| value.unwrap_or(false))
            .map_err(StoreError::database)
        })
    }

    fn request_cancellation(
        &self,
        job_id: Uuid,
        now: Timestamp,
    ) -> StoreFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            sqlx::query(concat!(
                "UPDATE job_outbox SET ",
                cancellation_assignments!(),
                " WHERE id = $2 AND status IN ('pending', 'running')"
            ))
            .bind(now.to_sqlx())
            .bind(job_id)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected() == 1)
            .map_err(StoreError::database)
        })
    }

    fn cancel<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            let mut connection = self.pool.acquire().await.map_err(StoreError::database)?;
            finish_on(
                &mut connection,
                job_id,
                worker_id,
                JobStatus::Cancelled,
                None,
                now,
            )
            .await
        })
    }

    fn heartbeat<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        lease_for: Duration,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            let locked_until = lease_deadline(now, lease_for)?;
            heartbeat_on(&self.pool, job_id, worker_id, locked_until, now).await
        })
    }

    fn release<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        run_after: Timestamp,
        reason: ReleaseReason,
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            release_on(&self.pool, job_id, worker_id, run_after, reason, now).await
        })
    }

    fn oldest_pending_age<'a>(
        &'a self,
        job_types: &'a [&'a str],
        now: Timestamp,
    ) -> StoreFuture<'a, Result<Duration, StoreError>> {
        Box::pin(async move {
            validate_job_types(job_types)?;
            oldest_ready_age_on(&self.pool, &owned_job_types(job_types), now).await
        })
    }

    fn queue_depth<'a>(
        &'a self,
        job_types: &'a [&'a str],
    ) -> StoreFuture<'a, Result<QueueDepth, StoreError>> {
        Box::pin(async move {
            validate_job_types(job_types)?;
            queue_depth_on(&self.pool, &owned_job_types(job_types)).await
        })
    }

    fn ready(&self) -> StoreFuture<'_, Result<(), StoreError>> {
        let pool = self.pool.clone();
        Box::pin(owned_operation(async move {
            let no_job_types: &[&str] = &[];
            let mut transaction = pool.begin().await.map_err(StoreError::database)?;
            sqlx::query(
                "SELECT id FROM job_outbox WHERE attempts < max_attempts AND cancel_requested_at IS NULL AND job_type = ANY($2) AND ((status = 'pending' AND run_after <= $1) OR (status = 'running' AND locked_until <= $1)) ORDER BY run_after, created_at, id FOR UPDATE SKIP LOCKED LIMIT 0",
            )
            .bind(jiff_sqlx::ToSqlx::to_sqlx(Timestamp::now()))
            .bind(no_job_types)
            .fetch_all(&mut *transaction)
            .await
            .map(|_| ())
            .map_err(StoreError::database)
        }))
    }
}

fn owned_job_types(job_types: &[&str]) -> Vec<String> {
    job_types.iter().map(|kind| (*kind).to_owned()).collect()
}

async fn owned_operation<T: Send + 'static>(
    operation: impl Future<Output = Result<T, StoreError>> + Send + 'static,
) -> Result<T, StoreError> {
    tokio::spawn(operation).await.map_err(StoreError::Task)?
}

async fn finish_on(
    connection: &mut PgConnection,
    job_id: Uuid,
    worker_id: &str,
    status: JobStatus,
    error: Option<&str>,
    now: Timestamp,
) -> Result<bool, StoreError> {
    let status = status_name(status);
    let allow_cancellation = status == "cancelled";
    sqlx::query(concat!(
        "UPDATE job_outbox SET status = $5, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = $6, failure_reason = NULL, updated_at = $3 WHERE id = $1 AND ",
        owned_lease_predicate!()
    ))
    .bind(job_id)
    .bind(worker_id)
    .bind(now.to_sqlx())
    .bind(allow_cancellation)
    .bind(status)
    .bind(error.map(bound_error))
    .execute(connection)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(StoreError::database)
}

async fn request_cancellation_by_key_on(
    connection: &mut PgConnection,
    job_type: &str,
    key: &str,
    now: Timestamp,
) -> Result<CancellationOutcome, StoreError> {
    let statuses: Vec<String> = sqlx::query_scalar(concat!(
        "UPDATE job_outbox SET ",
        cancellation_assignments!(),
        " WHERE job_type = $2 AND idempotency_key = $3 AND status IN ('pending', 'running') RETURNING status"
    ))
    .bind(now.to_sqlx())
    .bind(job_type)
    .bind(key)
    .fetch_all(connection)
    .await
    .map_err(StoreError::database)?;
    count_cancellation_statuses(statuses)
}

async fn request_cancellation_by_key_prefix_on(
    connection: &mut PgConnection,
    job_type: &str,
    prefix: &str,
    now: Timestamp,
) -> Result<CancellationOutcome, StoreError> {
    let statuses: Vec<String> = sqlx::query_scalar(concat!(
        "UPDATE job_outbox SET ",
        cancellation_assignments!(),
        " WHERE job_type = $2 AND left(idempotency_key, char_length($3)) = $3 AND status IN ('pending', 'running') RETURNING status"
    ))
    .bind(now.to_sqlx())
    .bind(job_type)
    .bind(prefix)
    .fetch_all(connection)
    .await
    .map_err(StoreError::database)?;
    count_cancellation_statuses(statuses)
}

fn count_cancellation_statuses(statuses: Vec<String>) -> Result<CancellationOutcome, StoreError> {
    let mut outcome = CancellationOutcome::default();
    for status in statuses {
        match status.as_str() {
            "cancelled" => outcome.cancelled_pending += 1,
            "running" => outcome.cancellation_requested_running += 1,
            value => {
                return Err(StoreError::InvalidData(format!(
                    "cancellation returned status `{value}`"
                )));
            }
        }
    }
    Ok(outcome)
}

fn row_to_job(row: &sqlx::postgres::PgRow) -> Result<Job, StoreError> {
    let attempts = row.try_get::<i32, _>("attempts")?;
    let max_attempts = row.try_get::<i32, _>("max_attempts")?;
    let deferrals = row.try_get::<i32, _>("deferrals")?;
    Ok(Job {
        id: row.try_get("id")?,
        job_type: row.try_get("job_type")?,
        payload: row.try_get("payload")?,
        status: parse_status(&row.try_get::<String, _>("status")?)?,
        attempts: u32::try_from(attempts)
            .map_err(|_| StoreError::InvalidData(format!("negative attempt count {attempts}")))?,
        max_attempts: u32::try_from(max_attempts).map_err(|_| {
            StoreError::InvalidData(format!("negative maximum attempt count {max_attempts}"))
        })?,
        run_after: row.try_get::<SqlTimestamp, _>("run_after")?.to_jiff(),
        locked_by: row.try_get("locked_by")?,
        locked_until: row
            .try_get::<Option<SqlTimestamp>, _>("locked_until")?
            .map(SqlTimestamp::to_jiff),
        idempotency_key: row.try_get("idempotency_key")?,
        dedupe_key: row.try_get("dedupe_key")?,
        pending_dedupe_key: row.try_get("pending_dedupe_key")?,
        last_error: row.try_get("last_error")?,
        failure_reason: row
            .try_get::<Option<String>, _>("failure_reason")?
            .as_deref()
            .map(parse_failure_reason)
            .transpose()?,
        cancel_requested_at: row
            .try_get::<Option<SqlTimestamp>, _>("cancel_requested_at")?
            .map(SqlTimestamp::to_jiff),
        deferrals: u32::try_from(deferrals)
            .map_err(|_| StoreError::InvalidData(format!("negative deferral count {deferrals}")))?,
        concurrency_key: row.try_get("concurrency_key")?,
        created_at: row.try_get::<SqlTimestamp, _>("created_at")?.to_jiff(),
        updated_at: row.try_get::<SqlTimestamp, _>("updated_at")?.to_jiff(),
    })
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::database(error)
    }
}

#[cfg(test)]
mod tests;
