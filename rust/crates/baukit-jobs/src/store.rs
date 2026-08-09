use std::{future::Future, pin::Pin, time::Duration};

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Postgres, Row as _, Transaction};
use uuid::Uuid;

use crate::{ClaimedJob, EnqueueOutcome, FailureDisposition, Job, JobStatus, NewJob, StoreError};

const MAX_JOB_TYPE_LENGTH: usize = 200;
const MAX_WORKER_ID_LENGTH: usize = 300;
const MAX_IDEMPOTENCY_KEY_LENGTH: usize = 500;
const MAX_ERROR_LENGTH: usize = 10_000;

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

    /// Claims the oldest ready job and increments its attempt count atomically.
    fn claim<'a>(
        &'a self,
        worker_id: &'a str,
        now: DateTime<Utc>,
        lease_for: Duration,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>>;

    /// Completes a currently owned job atomically.
    fn complete<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, Result<bool, StoreError>>;

    /// Records a failed attempt, retrying only when permitted and attempts remain.
    fn record_failure<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        retryable: bool,
        retry_at: DateTime<Utc>,
        error: &'a str,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>>;

    /// Returns whether cancellation was requested for a running job.
    fn cancellation_requested(&self, job_id: Uuid) -> StoreFuture<'_, Result<bool, StoreError>>;

    /// Requests cancellation, immediately cancelling pending jobs.
    fn request_cancellation(
        &self,
        job_id: Uuid,
        now: DateTime<Utc>,
    ) -> StoreFuture<'_, Result<bool, StoreError>>;

    /// Marks a currently owned job cancelled.
    fn cancel<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, Result<bool, StoreError>>;

    /// Returns the age of the oldest pending job, or zero for an empty queue.
    fn oldest_pending_age(
        &self,
        now: DateTime<Utc>,
    ) -> StoreFuture<'_, Result<Duration, StoreError>>;

    /// Probes the same locking query shape used to claim work without claiming it.
    fn ready(&self) -> StoreFuture<'_, Result<(), StoreError>>;
}

/// PostgreSQL [`JobStore`] backed by a product-owned `job_outbox` table.
#[derive(Clone, Debug)]
pub struct PostgresJobStore {
    pool: PgPool,
}

impl PostgresJobStore {
    /// Creates a store using an existing product pool.
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Returns the underlying pool for product-level transaction composition.
    pub const fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Enqueues inside a caller-owned transaction.
    ///
    /// Use this with a domain mutation to implement the transactional outbox
    /// pattern. The transaction remains owned by the caller.
    pub async fn enqueue_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job: NewJob,
    ) -> Result<EnqueueOutcome, StoreError> {
        validate_new_job(&job)?;
        enqueue_on(transaction, job).await
    }

    /// Marks a job successful inside a caller-owned transaction.
    ///
    /// Product writes performed earlier in the transaction and this lease-owned
    /// transition commit or roll back together.
    pub async fn complete_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        job_id: Uuid,
        worker_id: &str,
        now: DateTime<Utc>,
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
}

impl JobStore for PostgresJobStore {
    fn enqueue(&self, job: NewJob) -> StoreFuture<'_, Result<EnqueueOutcome, StoreError>> {
        Box::pin(async move {
            validate_new_job(&job)?;
            let mut transaction = self.pool.begin().await.map_err(StoreError::database)?;
            let outcome = enqueue_on(&mut transaction, job).await?;
            transaction.commit().await.map_err(StoreError::database)?;
            Ok(outcome)
        })
    }

    fn claim<'a>(
        &'a self,
        worker_id: &'a str,
        now: DateTime<Utc>,
        lease_for: Duration,
    ) -> StoreFuture<'a, Result<Option<ClaimedJob>, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            if lease_for.is_zero() {
                return Err(StoreError::InvalidInput(
                    "lease duration must be non-zero".to_owned(),
                ));
            }
            let lease_for = chrono::Duration::from_std(lease_for).map_err(|_| {
                StoreError::InvalidInput("lease duration is out of range".to_owned())
            })?;
            let locked_until = now.checked_add_signed(lease_for).ok_or_else(|| {
                StoreError::InvalidInput("lease deadline is out of range".to_owned())
            })?;
            let mut transaction = self.pool.begin().await.map_err(StoreError::database)?;

            sqlx::query(
                "UPDATE job_outbox SET status = 'cancelled', locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, updated_at = $1 WHERE status = 'running' AND locked_until <= $1 AND cancel_requested_at IS NOT NULL",
            )
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::database)?;
            sqlx::query(
                "UPDATE job_outbox SET status = 'failed', locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = COALESCE(last_error, 'worker lease expired after final attempt'), updated_at = $1 WHERE status = 'running' AND locked_until <= $1 AND attempts >= max_attempts",
            )
            .bind(now)
            .execute(&mut *transaction)
            .await
            .map_err(StoreError::database)?;

            let row = sqlx::query(
                "WITH candidate AS (SELECT id FROM job_outbox WHERE attempts < max_attempts AND cancel_requested_at IS NULL AND ((status = 'pending' AND run_after <= $1) OR (status = 'running' AND locked_until <= $1)) ORDER BY run_after, created_at, id FOR UPDATE SKIP LOCKED LIMIT 1) UPDATE job_outbox AS job SET status = 'running', attempts = job.attempts + 1, locked_by = $2, locked_until = $3, last_error = NULL, updated_at = $1 FROM candidate WHERE job.id = candidate.id RETURNING job.id, job.job_type, job.payload, job.status, job.attempts, job.max_attempts, job.run_after, job.locked_by, job.locked_until, job.idempotency_key, job.last_error, job.cancel_requested_at, job.created_at, job.updated_at",
            )
                .bind(now)
                .bind(worker_id)
                .bind(locked_until)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(StoreError::database)?;
            transaction.commit().await.map_err(StoreError::database)?;
            row.map(row_to_job).transpose()
        })
    }

    fn complete<'a>(
        &'a self,
        job_id: Uuid,
        worker_id: &'a str,
        now: DateTime<Utc>,
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
        retry_at: DateTime<Utc>,
        error: &'a str,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>> {
        Box::pin(async move {
            validate_worker_id(worker_id)?;
            let row = sqlx::query(
                "UPDATE job_outbox SET status = CASE WHEN $3 AND attempts < max_attempts THEN 'pending' ELSE 'failed' END, run_after = CASE WHEN $3 AND attempts < max_attempts THEN $4 ELSE run_after END, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = $5, updated_at = $6 WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $6 RETURNING status",
            )
            .bind(job_id)
            .bind(worker_id)
            .bind(retryable)
            .bind(retry_at)
            .bind(bound_error(error))
            .bind(now)
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
        now: DateTime<Utc>,
    ) -> StoreFuture<'_, Result<bool, StoreError>> {
        Box::pin(async move {
            sqlx::query(
                "UPDATE job_outbox SET status = CASE WHEN status = 'pending' THEN 'cancelled' ELSE status END, locked_by = CASE WHEN status = 'pending' THEN NULL ELSE locked_by END, locked_until = CASE WHEN status = 'pending' THEN NULL ELSE locked_until END, cancel_requested_at = CASE WHEN status = 'running' THEN $2 ELSE NULL END, updated_at = $2 WHERE id = $1 AND status IN ('pending', 'running')",
            )
            .bind(job_id)
            .bind(now)
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
        now: DateTime<Utc>,
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

    fn oldest_pending_age(
        &self,
        now: DateTime<Utc>,
    ) -> StoreFuture<'_, Result<Duration, StoreError>> {
        Box::pin(async move {
            let seconds = sqlx::query_scalar::<_, f64>(
                "SELECT COALESCE(GREATEST(0, EXTRACT(EPOCH FROM ($1 - MIN(created_at)))), 0)::double precision FROM job_outbox WHERE status = 'pending'",
            )
            .bind(now)
            .fetch_one(&self.pool)
            .await
            .map_err(StoreError::database)?;
            Ok(Duration::from_secs_f64(seconds))
        })
    }

    fn ready(&self) -> StoreFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let mut transaction = self.pool.begin().await.map_err(StoreError::database)?;
            sqlx::query(
                "SELECT id FROM job_outbox WHERE attempts < max_attempts AND cancel_requested_at IS NULL AND ((status = 'pending' AND run_after <= $1) OR (status = 'running' AND locked_until <= $1)) ORDER BY run_after, created_at, id FOR UPDATE SKIP LOCKED LIMIT 0",
            )
            .bind(Utc::now())
            .fetch_all(&mut *transaction)
            .await
            .map(|_| ())
            .map_err(StoreError::database)
        })
    }
}

async fn enqueue_on(
    connection: &mut PgConnection,
    job: NewJob,
) -> Result<EnqueueOutcome, StoreError> {
    let max_attempts = i32::try_from(job.max_attempts).map_err(|_| {
        StoreError::InvalidInput("max_attempts exceeds PostgreSQL INTEGER".to_owned())
    })?;
    let inserted = sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts, run_after, idempotency_key, created_at, updated_at) VALUES ($1, $2, $3, 'pending', 0, $4, $5, $6, $7, $7) ON CONFLICT (job_type, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING RETURNING id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, last_error, cancel_requested_at, created_at, updated_at",
    )
        .bind(job.id)
        .bind(&job.job_type)
        .bind(&job.payload)
        .bind(max_attempts)
        .bind(job.run_after)
        .bind(&job.idempotency_key)
        .bind(job.created_at)
        .fetch_optional(&mut *connection)
        .await
        .map_err(StoreError::database)?;
    if let Some(row) = inserted {
        return Ok(EnqueueOutcome {
            job: row_to_job(row)?,
            created: true,
        });
    }

    let key = job.idempotency_key.ok_or_else(|| {
        StoreError::InvalidData("non-idempotent enqueue unexpectedly conflicted".to_owned())
    })?;
    let row = sqlx::query(
        "SELECT id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, last_error, cancel_requested_at, created_at, updated_at FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2",
    )
        .bind(job.job_type)
        .bind(key)
        .fetch_one(connection)
        .await
        .map_err(StoreError::database)?;
    Ok(EnqueueOutcome {
        job: row_to_job(row)?,
        created: false,
    })
}

async fn finish_on(
    connection: &mut PgConnection,
    job_id: Uuid,
    worker_id: &str,
    status: JobStatus,
    error: Option<&str>,
    now: DateTime<Utc>,
) -> Result<bool, StoreError> {
    let status = status_name(status);
    let allow_cancellation = status == "cancelled";
    sqlx::query(
        "UPDATE job_outbox SET status = $3, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = $4, updated_at = $5 WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $5 AND ($6 OR cancel_requested_at IS NULL)",
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(status)
    .bind(error.map(bound_error))
    .bind(now)
    .bind(allow_cancellation)
    .execute(connection)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(StoreError::database)
}

fn row_to_job(row: sqlx::postgres::PgRow) -> Result<Job, StoreError> {
    let attempts = row.try_get::<i32, _>("attempts")?;
    let max_attempts = row.try_get::<i32, _>("max_attempts")?;
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
        run_after: row.try_get("run_after")?,
        locked_by: row.try_get("locked_by")?,
        locked_until: row.try_get("locked_until")?,
        idempotency_key: row.try_get("idempotency_key")?,
        last_error: row.try_get("last_error")?,
        cancel_requested_at: row.try_get("cancel_requested_at")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn validate_new_job(job: &NewJob) -> Result<(), StoreError> {
    validate_text("job_type", &job.job_type, MAX_JOB_TYPE_LENGTH)?;
    if job.max_attempts == 0 {
        return Err(StoreError::InvalidInput(
            "max_attempts must be greater than zero".to_owned(),
        ));
    }
    if let Some(key) = &job.idempotency_key {
        validate_text("idempotency_key", key, MAX_IDEMPOTENCY_KEY_LENGTH)?;
    }
    Ok(())
}

fn validate_worker_id(worker_id: &str) -> Result<(), StoreError> {
    validate_text("worker_id", worker_id, MAX_WORKER_ID_LENGTH)
}

fn validate_text(name: &str, value: &str, max: usize) -> Result<(), StoreError> {
    let length = value.chars().count();
    if value.trim().is_empty() || length > max {
        Err(StoreError::InvalidInput(format!(
            "{name} must contain 1 to {max} characters"
        )))
    } else {
        Ok(())
    }
}

fn bound_error(error: &str) -> String {
    let bounded = error.trim();
    if bounded.is_empty() {
        "job failed without an error message".to_owned()
    } else {
        bounded.chars().take(MAX_ERROR_LENGTH).collect()
    }
}

fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

fn parse_status(status: &str) -> Result<JobStatus, StoreError> {
    match status {
        "pending" => Ok(JobStatus::Pending),
        "running" => Ok(JobStatus::Running),
        "succeeded" => Ok(JobStatus::Succeeded),
        "failed" => Ok(JobStatus::Failed),
        "cancelled" => Ok(JobStatus::Cancelled),
        value => Err(StoreError::InvalidData(format!(
            "unknown job status `{value}`"
        ))),
    }
}

impl From<sqlx::Error> for StoreError {
    fn from(error: sqlx::Error) -> Self {
        Self::database(error)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn new_job_defaults_are_claimable_and_unique() {
        let job = NewJob::new("email.send", json!({"message_id": 7}), 3);
        assert_eq!(job.job_type, "email.send");
        assert_eq!(job.max_attempts, 3);
        assert!(job.idempotency_key.is_none());
        assert!(job.run_after >= job.created_at);
    }

    #[test]
    fn error_text_is_bounded_on_character_boundaries() {
        let error = "é".repeat(MAX_ERROR_LENGTH + 1);
        assert_eq!(bound_error(&error).chars().count(), MAX_ERROR_LENGTH);
    }
}
