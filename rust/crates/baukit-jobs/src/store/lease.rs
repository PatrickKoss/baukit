use std::time::Duration;

use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{FailureDisposition, ReleaseReason, StoreError};

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
    job_id: Uuid,
    worker_id: &str,
    locked_until: Timestamp,
    now: Timestamp,
) -> Result<bool, StoreError> {
    sqlx::query(
        "UPDATE job_outbox SET locked_until = $4 WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $3",
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(now.to_sqlx())
    .bind(locked_until.to_sqlx())
    .execute(pool)
    .await
    .map(|result| result.rows_affected() == 1)
    .map_err(StoreError::database)
}

pub(super) async fn release_on(
    pool: &PgPool,
    job_id: Uuid,
    worker_id: &str,
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
         WHERE id = $1 AND status = 'running' AND locked_by = $2 AND locked_until > $3 AND cancel_requested_at IS NULL \
         RETURNING status",
    )
    .bind(job_id)
    .bind(worker_id)
    .bind(now.to_sqlx())
    .bind(run_after.to_sqlx())
    .bind(reason.consumes_attempt())
    .bind(reason.counts_deferral())
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
