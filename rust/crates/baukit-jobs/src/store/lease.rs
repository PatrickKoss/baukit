use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use super::validation::status_name;
use crate::{FailureDisposition, JobLease, JobStatus, ReleaseReason, StoreError};

macro_rules! owned_lease_predicate {
    ($attempt:literal) => {
        concat!(
            "status = 'running' AND locked_by = $2 AND locked_until > $3 AND ($4 OR cancel_requested_at IS NULL) AND attempts = ",
            $attempt
        )
    };
}

pub(super) fn lease_deadline(now: Timestamp, lease_for: Duration) -> Result<Timestamp, StoreError> {
    if lease_for.is_zero() {
        return Err(StoreError::InvalidInput(
            "lease duration must be non-zero".to_owned(),
        ));
    }
    let lease_for = SignedDuration::try_from(lease_for)
        .map_err(|_| StoreError::InvalidInput("lease duration is out of range".to_owned()))?;
    now.checked_add(lease_for)
        .map_err(|_| StoreError::InvalidInput("lease deadline is out of range".to_owned()))
}

pub(super) async fn heartbeat_on(
    pool: &PgPool,
    key: LeaseKey<'_>,
    locked_until: Timestamp,
    now: Timestamp,
) -> Result<bool, StoreError> {
    sqlx::query(
        "UPDATE job_outbox SET locked_until = $4 WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $3 AND attempts = $5",
    )
    .bind(key.job_id)
    .bind(key.worker_id)
    .bind(now.to_sqlx())
    .bind(locked_until.to_sqlx())
    .bind(key.attempt)
    .execute(pool)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(StoreError::database)
}

pub(super) async fn release_on(
    pool: &PgPool,
    key: LeaseKey<'_>,
    run_after: Timestamp,
    reason: ReleaseReason,
    now: Timestamp,
) -> Result<Option<FailureDisposition>, StoreError> {
    let status = sqlx::query_scalar::<_, String>(
        "UPDATE job_outbox SET \
         status = CASE WHEN $5 AND attempts >= max_attempts THEN 'failed' ELSE 'pending' END, \
         run_after = CASE WHEN $5 AND attempts >= max_attempts THEN run_after ELSE $4 END, \
         attempts = CASE WHEN $5 THEN attempts ELSE GREATEST(attempts - 1, 0) END, \
         deferrals = CASE WHEN $6 THEN deferrals + 1 ELSE deferrals END, \
         failure_reason = CASE WHEN $5 AND attempts >= max_attempts THEN 'attempts_exhausted' ELSE NULL END, \
         last_error = CASE WHEN $5 AND attempts >= max_attempts THEN COALESCE(last_error, 'deferral consumed the final attempt') ELSE last_error END, \
         locked_by = NULL, locked_until = NULL, updated_at = $3 \
         WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $3 AND cancel_requested_at IS NULL AND attempts = $7 \
         RETURNING status",
    )
    .bind(key.job_id)
    .bind(key.worker_id)
    .bind(now.to_sqlx())
    .bind(run_after.to_sqlx())
    .bind(reason.consumes_attempt())
    .bind(reason.counts_deferral())
    .bind(key.attempt)
    .fetch_optional(pool)
    .await
    .map_err(StoreError::database)?;
    status
        .map(|status| match status.as_str() {
            "pending" => Ok(FailureDisposition::Retry),
            "failed" => Ok(FailureDisposition::Failed),
            value => Err(StoreError::InvalidData(format!(
                "release returned status `{value}`"
            ))),
        })
        .transpose()
}

/// Lease identity bound into an owned write.
pub(super) struct LeaseKey<'a> {
    pub(super) job_id: Uuid,
    pub(super) worker_id: &'a str,
    pub(super) attempt: i32,
}

impl<'a> LeaseKey<'a> {
    pub(super) fn attempt(lease: &'a JobLease) -> Result<Self, StoreError> {
        Ok(Self {
            job_id: lease.job_id,
            worker_id: &lease.worker_id,
            attempt: attempt_param(lease.attempt)?,
        })
    }
}

pub(super) fn attempt_param(attempt: u32) -> Result<i32, StoreError> {
    i32::try_from(attempt)
        .map_err(|_| StoreError::InvalidInput("attempt exceeds PostgreSQL INTEGER".to_owned()))
}

pub(super) async fn check_lease_on(
    connection: &mut PgConnection,
    key: LeaseKey<'_>,
    now: Timestamp,
) -> Result<bool, StoreError> {
    sqlx::query_scalar::<_, Uuid>(concat!(
        "SELECT id FROM job_outbox WHERE id = $1 AND ",
        owned_lease_predicate!("$5"),
        " FOR UPDATE"
    ))
    .bind(key.job_id)
    .bind(key.worker_id)
    .bind(now.to_sqlx())
    .bind(false)
    .bind(key.attempt)
    .fetch_optional(connection)
    .await
    .map(|row| row.is_some())
    .map_err(StoreError::database)
}

pub(super) async fn finish_on(
    connection: &mut PgConnection,
    key: LeaseKey<'_>,
    status: JobStatus,
    now: Timestamp,
) -> Result<bool, StoreError> {
    let status = status_name(status);
    let allow_cancellation = status == "cancelled";
    sqlx::query(concat!(
        "UPDATE job_outbox SET status = $6, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = NULL, failure_reason = NULL, updated_at = $3 WHERE id = $1 AND ",
        owned_lease_predicate!("$5")
    ))
    .bind(key.job_id)
    .bind(key.worker_id)
    .bind(now.to_sqlx())
    .bind(allow_cancellation)
    .bind(key.attempt)
    .bind(status)
    .execute(connection)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(StoreError::database)
}
