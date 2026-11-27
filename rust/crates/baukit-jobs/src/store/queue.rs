use std::time::Duration;

use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use sqlx::PgPool;

use crate::{QueueDepth, StoreError};

pub(super) async fn oldest_ready_age_on(
    pool: &PgPool,
    job_types: &[String],
    now: Timestamp,
) -> Result<Duration, StoreError> {
    let seconds = sqlx::query_scalar::<_, f64>(
        "SELECT COALESCE(GREATEST(0, EXTRACT(EPOCH FROM ($1 - MIN(run_after)))), 0)::double precision FROM job_outbox WHERE status = 'pending' AND run_after <= $1 AND job_type = ANY($2)",
    )
    .bind(now.to_sqlx())
    .bind(job_types)
    .fetch_one(pool)
    .await
    .map_err(StoreError::database)?;
    Duration::try_from_secs_f64(seconds)
        .map_err(|_| StoreError::InvalidData(format!("queue age `{seconds}` is out of range")))
}

pub(super) async fn queue_depth_on(
    pool: &PgPool,
    job_types: &[String],
) -> Result<QueueDepth, StoreError> {
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT status, count(*) FROM job_outbox WHERE job_type = ANY($1) AND status IN ('pending', 'running', 'failed') GROUP BY status",
    )
    .bind(job_types)
    .fetch_all(pool)
    .await
    .map_err(StoreError::database)?;
    let mut depth = QueueDepth::default();
    for (status, count) in rows {
        let count = u64::try_from(count)
            .map_err(|_| StoreError::InvalidData(format!("negative `{status}` row count")))?;
        match status.as_str() {
            "pending" => depth.pending = count,
            "running" => depth.running = count,
            "failed" => depth.failed = count,
            value => {
                return Err(StoreError::InvalidData(format!(
                    "queue depth returned status `{value}`"
                )));
            }
        }
    }
    Ok(depth)
}
