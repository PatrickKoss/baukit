use std::num::NonZeroU32;

use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use sqlx::{PgConnection, Row as _};
use uuid::Uuid;

use crate::{ClaimedJob, StoreError};

use super::row_to_job;

/// Candidates whose key turned out to be full or locked by a concurrent claim
/// before the claim gives up and reports an empty queue for this poll.
const MAX_KEY_PROBES: usize = 4;
const KEY_LOCK_NAMESPACE: &str = "baukit-jobs:concurrency:";

const RECLAIM_CANCELLED_SQL: &str = "UPDATE job_outbox SET status = 'cancelled', locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, updated_at = $1 WHERE status = 'running' AND locked_until <= $1 AND cancel_requested_at IS NOT NULL";

const FAIL_EXPIRED_FINAL_SQL: &str = "UPDATE job_outbox SET status = 'failed', locked_by = NULL, locked_until = NULL, cancel_requested_at = NULL, last_error = COALESCE(last_error, 'worker lease expired after final attempt'), failure_reason = 'attempts_exhausted', updated_at = $1 WHERE status = 'running' AND locked_until <= $1 AND attempts >= max_attempts";

/// Oldest claimable row of the handler's types. With a key limit (`$2`), rows
/// whose key already holds that many live leases are skipped.
pub(super) const CANDIDATE_SQL: &str = "SELECT c.id, c.concurrency_key FROM job_outbox AS c WHERE c.attempts < c.max_attempts AND c.cancel_requested_at IS NULL AND c.job_type = ANY($3) AND NOT (c.id = ANY($4)) AND ((c.status = 'pending' AND c.run_after <= $1) OR (c.status = 'running' AND c.locked_until <= $1)) AND ($2::integer IS NULL OR c.concurrency_key IS NULL OR (SELECT count(*) FROM job_outbox AS r WHERE r.concurrency_key = c.concurrency_key AND r.status = 'running' AND r.locked_until > $1) < $2) ORDER BY c.run_after, c.created_at, c.id FOR UPDATE OF c SKIP LOCKED LIMIT 1";

const LIVE_LEASES_FOR_KEY_SQL: &str = "SELECT count(*) FROM job_outbox WHERE concurrency_key = $1 AND status = 'running' AND locked_until > $2";

const TRY_KEY_LOCK_SQL: &str = "SELECT pg_try_advisory_xact_lock(hashtextextended($1, 0))";

const TAKE_SQL: &str = "UPDATE job_outbox AS job SET status = 'running', pending_dedupe_key = NULL, attempts = job.attempts + 1, locked_by = $2, locked_until = $3, last_error = NULL, failure_reason = NULL, updated_at = $1 WHERE job.id = $4 RETURNING job.id, job.job_type, job.payload, job.status, job.attempts, job.max_attempts, job.run_after, job.locked_by, job.locked_until, job.idempotency_key, job.dedupe_key, job.pending_dedupe_key, job.last_error, job.failure_reason, job.cancel_requested_at, job.deferrals, job.concurrency_key, job.created_at, job.updated_at";

pub(super) struct ClaimRequest<'a> {
    pub(super) worker_id: &'a str,
    pub(super) job_types: &'a [String],
    pub(super) now: Timestamp,
    pub(super) locked_until: Timestamp,
    pub(super) max_per_key: Option<NonZeroU32>,
}

/// Runs the claim inside the caller's transaction. Rows locked while probing
/// stay locked until it commits.
pub(super) async fn claim_on(
    connection: &mut PgConnection,
    request: &ClaimRequest<'_>,
) -> Result<Option<ClaimedJob>, StoreError> {
    for sql in [RECLAIM_CANCELLED_SQL, FAIL_EXPIRED_FINAL_SQL] {
        sqlx::query(sql)
            .bind(request.now.to_sqlx())
            .execute(&mut *connection)
            .await?;
    }
    let limit = request.max_per_key.map(limit_param).transpose()?;
    let mut skipped: Vec<Uuid> = Vec::new();
    while skipped.len() < MAX_KEY_PROBES {
        let Some((id, key)) = next_candidate(connection, request, limit, &skipped).await? else {
            return Ok(None);
        };
        let admitted = match (limit, key) {
            (Some(limit), Some(key)) => key_has_room(connection, &key, limit, request.now).await?,
            _ => true,
        };
        if admitted {
            return take(connection, request, id).await.map(Some);
        }
        skipped.push(id);
    }
    Ok(None)
}

fn limit_param(limit: NonZeroU32) -> Result<i32, StoreError> {
    i32::try_from(limit.get())
        .map_err(|_| StoreError::InvalidInput("max_per_key exceeds PostgreSQL INTEGER".to_owned()))
}

async fn next_candidate(
    connection: &mut PgConnection,
    request: &ClaimRequest<'_>,
    limit: Option<i32>,
    skipped: &[Uuid],
) -> Result<Option<(Uuid, Option<String>)>, StoreError> {
    let row = sqlx::query(CANDIDATE_SQL)
        .bind(request.now.to_sqlx())
        .bind(limit)
        .bind(request.job_types)
        .bind(skipped)
        .fetch_optional(&mut *connection)
        .await?;
    row.map(|row| Ok((row.try_get("id")?, row.try_get("concurrency_key")?)))
        .transpose()
}

/// Serializes claims per key with a transaction-scoped advisory lock, then
/// recounts live leases so two concurrent claims cannot both take the last slot.
/// A key locked by another claim counts as full for this poll.
async fn key_has_room(
    connection: &mut PgConnection,
    key: &str,
    limit: i32,
    now: Timestamp,
) -> Result<bool, StoreError> {
    let locked: bool = sqlx::query_scalar(TRY_KEY_LOCK_SQL)
        .bind(format!("{KEY_LOCK_NAMESPACE}{key}"))
        .fetch_one(&mut *connection)
        .await?;
    if !locked {
        return Ok(false);
    }
    let running: i64 = sqlx::query_scalar(LIVE_LEASES_FOR_KEY_SQL)
        .bind(key)
        .bind(now.to_sqlx())
        .fetch_one(&mut *connection)
        .await?;
    Ok(running < i64::from(limit))
}

async fn take(
    connection: &mut PgConnection,
    request: &ClaimRequest<'_>,
    id: Uuid,
) -> Result<ClaimedJob, StoreError> {
    let row = sqlx::query(TAKE_SQL)
        .bind(request.now.to_sqlx())
        .bind(request.worker_id)
        .bind(request.locked_until.to_sqlx())
        .bind(id)
        .fetch_one(connection)
        .await?;
    row_to_job(&row)
}
