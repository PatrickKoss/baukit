use jiff_sqlx::ToSqlx;
use sqlx::PgConnection;

use crate::{EnqueueOutcome, NewJob, StoreError};

use super::{Deduplication, listener::notify_enqueued, row_to_job};

pub(super) async fn enqueue_on(
    connection: &mut PgConnection,
    job: NewJob,
) -> Result<EnqueueOutcome, StoreError> {
    let max_attempts = i32::try_from(job.max_attempts).map_err(|_| {
        StoreError::InvalidInput("max_attempts exceeds PostgreSQL INTEGER".to_owned())
    })?;
    let inserted = sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts, run_after, idempotency_key, concurrency_key, created_at, updated_at) VALUES ($1, $2, $3, 'pending', 0, $4, $5, $6, $8, $7, $7) ON CONFLICT (job_type, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING RETURNING id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at",
    )
        .bind(job.id)
        .bind(&job.job_type)
        .bind(&job.payload)
        .bind(max_attempts)
        .bind(job.run_after.to_sqlx())
        .bind(&job.idempotency_key)
        .bind(job.created_at.to_sqlx())
        .bind(&job.concurrency_key)
        .fetch_optional(&mut *connection)
        .await
        .map_err(StoreError::database)?;
    if let Some(row) = inserted {
        notify_enqueued(connection, &job.job_type).await?;
        return Ok(EnqueueOutcome {
            job: row_to_job(&row)?,
            created: true,
        });
    }

    let key = job.idempotency_key.ok_or_else(|| {
        StoreError::InvalidData("non-idempotent enqueue unexpectedly conflicted".to_owned())
    })?;
    let row = sqlx::query(
        "SELECT id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at FROM job_outbox WHERE job_type = $1 AND idempotency_key = $2",
    )
        .bind(job.job_type)
        .bind(key)
        .fetch_one(connection)
        .await
        .map_err(StoreError::database)?;
    Ok(EnqueueOutcome {
        job: row_to_job(&row)?,
        created: false,
    })
}

pub(super) async fn enqueue_deduplicated_on(
    connection: &mut PgConnection,
    job: NewJob,
    dedupe_key: &str,
    deduplication: Deduplication,
) -> Result<EnqueueOutcome, StoreError> {
    let max_attempts = i32::try_from(job.max_attempts).map_err(|_| {
        StoreError::InvalidInput("max_attempts exceeds PostgreSQL INTEGER".to_owned())
    })?;
    let insert_sql = match deduplication {
        Deduplication::Active => {
            "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts, run_after, dedupe_key, concurrency_key, created_at, updated_at) VALUES ($1, $2, $3, 'pending', 0, $4, $5, $6, $8, $7, $7) ON CONFLICT (job_type, dedupe_key) WHERE status IN ('pending', 'running') AND dedupe_key IS NOT NULL DO NOTHING RETURNING id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at"
        }
        Deduplication::Pending => {
            "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts, run_after, pending_dedupe_key, concurrency_key, created_at, updated_at) VALUES ($1, $2, $3, 'pending', 0, $4, $5, $6, $8, $7, $7) ON CONFLICT (job_type, pending_dedupe_key) WHERE status = 'pending' AND pending_dedupe_key IS NOT NULL DO NOTHING RETURNING id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at"
        }
    };
    let select_sql = match deduplication {
        Deduplication::Active => {
            "SELECT id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at FROM job_outbox WHERE job_type = $1 AND dedupe_key = $2 AND status IN ('pending', 'running') AND dedupe_key IS NOT NULL FOR UPDATE"
        }
        Deduplication::Pending => {
            "SELECT id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, idempotency_key, dedupe_key, pending_dedupe_key, last_error, failure_reason, cancel_requested_at, deferrals, concurrency_key, created_at, updated_at FROM job_outbox WHERE job_type = $1 AND pending_dedupe_key = $2 AND status = 'pending' AND pending_dedupe_key IS NOT NULL FOR UPDATE"
        }
    };
    loop {
        let inserted = sqlx::query(insert_sql)
            .bind(job.id)
            .bind(&job.job_type)
            .bind(&job.payload)
            .bind(max_attempts)
            .bind(job.run_after.to_sqlx())
            .bind(dedupe_key)
            .bind(job.created_at.to_sqlx())
            .bind(&job.concurrency_key)
            .fetch_optional(&mut *connection)
            .await
            .map_err(StoreError::database)?;
        if let Some(row) = inserted {
            notify_enqueued(connection, &job.job_type).await?;
            return Ok(EnqueueOutcome {
                job: row_to_job(&row)?,
                created: true,
            });
        }

        // Use a fresh statement snapshot to see a concurrent insert's commit.
        // If that row left the deduplicated statuses meanwhile, retry the
        // insert. Lock an existing row until this transaction commits.
        let existing = sqlx::query(select_sql)
            .bind(&job.job_type)
            .bind(dedupe_key)
            .fetch_optional(&mut *connection)
            .await
            .map_err(StoreError::database)?;
        if let Some(row) = existing {
            return Ok(EnqueueOutcome {
                job: row_to_job(&row)?,
                created: false,
            });
        }
    }
}
