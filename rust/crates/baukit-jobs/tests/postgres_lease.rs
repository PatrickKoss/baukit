#![cfg(test)]
use std::{error::Error, path::PathBuf, time::Duration};

use baukit_jobs::{
    FailureDisposition, JobLease, JobStatus, JobStore as _, LeaseRow, NewJob, PostgresJobStore,
    lease_predicate_holds,
};
use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const WORKER: &str = "shared-worker";
const LEASE: Duration = Duration::from_secs(30);

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn stale_attempt_of_the_same_worker_loses_the_lease() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store
        .enqueue(NewJob::new("lease.race", json!({}), 3))
        .await?
        .job;
    let start = Timestamp::now();
    let first = store
        .claim(WORKER, &["lease.race"], start, LEASE)
        .await?
        .ok_or("first claim")?;
    let after_expiry = start + SignedDuration::from_mins(1);
    let second = store
        .claim(WORKER, &["lease.race"], after_expiry, LEASE)
        .await?
        .ok_or("sibling slot reclaims")?;
    assert_eq!((first.id, first.attempts, second.attempts), (job.id, 1, 2));

    let stale = JobLease::of(&first, WORKER);
    let current = JobLease::of(&second, WORKER);
    assert!(!store.complete(&stale, after_expiry).await?);
    assert!(!store.cancel(&stale, after_expiry).await?);
    assert_eq!(
        store
            .record_failure(&stale, true, after_expiry, "stale", after_expiry)
            .await?,
        None
    );
    let mut transaction = pool.begin().await?;
    assert!(
        !store
            .check_lease_held_in_transaction(&mut transaction, &stale, after_expiry)
            .await?
    );
    assert!(
        !store
            .complete_lease_in_transaction(&mut transaction, &stale, after_expiry)
            .await?
    );
    transaction.rollback().await?;

    assert!(store.complete(&current, after_expiry).await?);
    let status: String = sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(job.id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "succeeded");
    assert_ne!(
        store
            .record_failure(&current, true, after_expiry, "late", after_expiry)
            .await?,
        Some(FailureDisposition::Retry)
    );
    pool.close().await;
    drop(fixture);
    Ok(())
}

/// K-17 pairing: the SQL predicate and [`lease_predicate_holds`] agree on every
/// generated row and caller.
#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn sql_lease_predicate_agrees_with_the_mirror() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, _store) = fixture().await?;
    let now = Timestamp::now();
    let times = [
        None,
        Some(now - SignedDuration::from_secs(1)),
        Some(now),
        Some(now + SignedDuration::from_secs(1)),
    ];
    let mut rows = Vec::new();
    for status in [JobStatus::Pending, JobStatus::Running, JobStatus::Succeeded] {
        for locked_by in [None, Some("worker-a"), Some("worker-b")] {
            for attempts in [1_u32, 2] {
                for locked_until in times {
                    for cancel_requested in [false, true] {
                        let running = status == JobStatus::Running;
                        let locked = locked_by.is_some() && locked_until.is_some();
                        let unlocked = locked_by.is_none() && locked_until.is_none();
                        if (cancel_requested && !running)
                            || (running && !locked)
                            || (!running && !unlocked)
                        {
                            continue;
                        }
                        let row = LeaseRow {
                            status,
                            locked_by,
                            attempts,
                            locked_until,
                            cancel_requested,
                        };
                        rows.push(row);
                    }
                }
            }
        }
    }
    for worker in ["worker-a", "worker-b"] {
        for attempt in [1_u32, 2, 3] {
            for allow_cancellation in [false, true] {
                for row in &rows {
                    let id = insert_row(&pool, row, now).await?;
                    let sql =
                        sql_predicate(&pool, id, worker, attempt, now, allow_cancellation).await?;
                    let mirror =
                        lease_predicate_holds(row, worker, attempt, now, allow_cancellation);
                    assert_eq!(
                        sql, mirror,
                        "{row:?} {worker} {attempt} {allow_cancellation}"
                    );
                }
            }
        }
    }
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn insert_row(
    pool: &PgPool,
    row: &LeaseRow<'_>,
    now: Timestamp,
) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::now_v7();
    let status = match row.status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        _ => "succeeded",
    };
    sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts, run_after, locked_by, locked_until, cancel_requested_at, created_at, updated_at) VALUES ($1, 'mirror', '{}', $2, $3, 5, $4, $5, $6, CASE WHEN $7 THEN $4 ELSE NULL END, $4, $4)",
    )
    .bind(id)
    .bind(status)
    .bind(i32::try_from(row.attempts).unwrap_or(i32::MAX))
    .bind(now.to_sqlx())
    .bind(row.locked_by)
    .bind(row.locked_until.map(|until| until.to_sqlx()))
    .bind(row.cancel_requested)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Uses the store's lease-bound check so the production SQL is what runs.
async fn sql_predicate(
    pool: &PgPool,
    id: Uuid,
    worker: &str,
    attempt: u32,
    now: Timestamp,
    allow_cancellation: bool,
) -> Result<bool, Box<dyn Error>> {
    let store = PostgresJobStore::new(pool.clone());
    let lease = JobLease::new(id, worker, attempt);
    let mut transaction = pool.begin().await?;
    let holds = if allow_cancellation {
        store.cancel(&lease, now).await?
    } else {
        store
            .check_lease_held_in_transaction(&mut transaction, &lease, now)
            .await?
    };
    transaction.rollback().await?;
    sqlx::query("DELETE FROM job_outbox WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(holds)
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}
