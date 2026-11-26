use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use sqlx::PgConnection;
use uuid::Uuid;

use crate::StoreError;

use super::{MAX_JOB_TYPE_LENGTH, listener::notify_enqueued, validation::validate_text};

/// Largest number of ids one [`PostgresJobStore::requeue_failed`](super::PostgresJobStore::requeue_failed) call accepts.
pub const MAX_REQUEUE_BATCH_SIZE: usize = 1000;

/// Failed rows of the type among the ids. A row whose active dedupe key is
/// held by a pending or running row stays failed, and of several failed rows
/// sharing a dedupe key only the newest is requeued.
const REQUEUE_SQL: &str = "WITH candidates AS (SELECT DISTINCT ON (COALESCE(f.dedupe_key, f.id::text)) f.id FROM job_outbox AS f WHERE f.job_type = $1 AND f.id = ANY($2) AND f.status = 'failed' AND NOT EXISTS (SELECT 1 FROM job_outbox AS a WHERE a.job_type = f.job_type AND a.dedupe_key = f.dedupe_key AND a.status IN ('pending', 'running')) ORDER BY COALESCE(f.dedupe_key, f.id::text), f.updated_at DESC, f.id) UPDATE job_outbox AS job SET status = 'pending', attempts = 0, deferrals = 0, run_after = $3, locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = NULL, failure_reason = NULL, updated_at = $3 FROM candidates WHERE job.id = candidates.id AND job.status = 'failed' RETURNING job.id";

pub(super) fn validate_requeue(job_type: &str, ids: &[Uuid]) -> Result<(), StoreError> {
    validate_text("job_type", job_type, MAX_JOB_TYPE_LENGTH)?;
    if ids.is_empty() || ids.len() > MAX_REQUEUE_BATCH_SIZE {
        return Err(StoreError::InvalidInput(format!(
            "requeue takes between 1 and {MAX_REQUEUE_BATCH_SIZE} job ids"
        )));
    }
    Ok(())
}

pub(super) async fn requeue_failed_on(
    connection: &mut PgConnection,
    job_type: &str,
    ids: &[Uuid],
    now: Timestamp,
) -> Result<Vec<Uuid>, StoreError> {
    let requeued: Vec<Uuid> = sqlx::query_scalar(REQUEUE_SQL)
        .bind(job_type)
        .bind(ids)
        .bind(now.to_sqlx())
        .fetch_all(&mut *connection)
        .await?;
    if !requeued.is_empty() {
        notify_enqueued(connection, job_type).await?;
    }
    Ok(requeued)
}
