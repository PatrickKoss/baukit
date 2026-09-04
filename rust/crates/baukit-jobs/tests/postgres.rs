use std::{collections::HashSet, error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{
    FailureDisposition, FixedUtcInterval, JobFailureReason, JobStatus, JobStore as _,
    MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE, NewJob, POSTGRES_MIGRATION_0002_SQL,
    POSTGRES_MIGRATION_SQL, PostgresJobStore, StoreError, TerminalJobCleanupOutcome,
    TerminalJobCutoffs,
};
use chrono::{TimeDelta, Utc};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Barrier;

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_idempotency_returns_the_original_job() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let first = NewJob::new("email.send", json!({"recipient": 1}), 3).idempotency_key("message:7");
    let original_id = first.id;

    let inserted = store.enqueue(first).await?;
    let duplicate = store
        .enqueue(
            NewJob::new("email.send", json!({"recipient": 999}), 8).idempotency_key("message:7"),
        )
        .await?;
    let other_type = store
        .enqueue(NewJob::new("sms.send", json!({}), 3).idempotency_key("message:7"))
        .await?;

    assert!(inserted.created);
    assert!(!duplicate.created);
    assert_eq!(duplicate.job.id, original_id);
    assert_eq!(duplicate.job.payload, json!({"recipient": 1}));
    assert!(other_type.created, "keys are scoped by job type");
    let mut transaction = pool.begin().await?;
    store
        .enqueue_in_transaction(
            &mut transaction,
            NewJob::new("rolled.back", json!({}), 3).idempotency_key("rollback"),
        )
        .await?;
    transaction.rollback().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 2);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_claim_is_concurrent_skip_locked_and_increments_attempts()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let store = Arc::new(store);
    for sequence in 0..8 {
        store
            .enqueue(NewJob::new("batch.item", json!({"sequence": sequence}), 3))
            .await?;
    }
    let now = Utc::now();

    let barrier = Arc::new(Barrier::new(17));
    let mut tasks = Vec::new();
    for worker in 0..16 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .claim(&format!("worker-{worker}"), now, Duration::from_secs(30))
                .await
        }));
    }
    barrier.wait().await;

    let mut claimed_ids = HashSet::new();
    for task in tasks {
        if let Some(job) = task.await?? {
            assert_eq!(job.status, JobStatus::Running);
            assert_eq!(job.attempts, 1);
            assert!(claimed_ids.insert(job.id), "a job was claimed twice");
        }
    }
    assert_eq!(claimed_ids.len(), 8);
    let attempts: Vec<i32> = sqlx::query_scalar("SELECT attempts FROM job_outbox")
        .fetch_all(&pool)
        .await?;
    assert!(attempts.iter().all(|attempts| *attempts == 1));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_expired_lease_is_reclaimed_without_a_sweep_process() -> Result<(), Box<dyn Error>>
{
    let (fixture, pool, store) = fixture().await?;
    let job = store
        .enqueue(NewJob::new("lease.test", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();

    let first = store
        .claim("worker-a", now, Duration::from_secs(1))
        .await?
        .expect("first claim");
    assert_eq!(first.id, job.id);
    assert!(
        store
            .claim("worker-b", now, Duration::from_secs(10))
            .await?
            .is_none()
    );
    let reclaimed = store
        .claim(
            "worker-b",
            now + TimeDelta::seconds(2),
            Duration::from_secs(10),
        )
        .await?
        .expect("expired lease is claimable");
    assert_eq!(reclaimed.id, job.id);
    assert_eq!(reclaimed.attempts, 2);
    assert_eq!(reclaimed.locked_by.as_deref(), Some("worker-b"));

    assert!(
        store
            .complete(reclaimed.id, "worker-b", now + TimeDelta::seconds(2))
            .await?
    );
    let exhausted = store
        .enqueue(NewJob::new("lease.exhausted", json!({}), 1))
        .await?
        .job;
    let exhausted_now = Utc::now();
    store
        .claim("worker-c", exhausted_now, Duration::from_secs(1))
        .await?
        .expect("final attempt claimed");
    assert!(
        store
            .claim(
                "worker-d",
                exhausted_now + TimeDelta::seconds(2),
                Duration::from_secs(10),
            )
            .await?
            .is_none(),
        "an expired final attempt is not reclaimed"
    );
    assert_eq!(status(&pool, exhausted.id).await?, "failed");
    assert_eq!(
        failure_reason(&pool, exhausted.id).await?.as_deref(),
        Some("attempts_exhausted")
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_cancellation_covers_pending_and_running_lifecycle() -> Result<(), Box<dyn Error>>
{
    let (fixture, pool, store) = fixture().await?;
    let pending = store
        .enqueue(NewJob::new("cancel.pending", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    assert!(store.request_cancellation(pending.id, now).await?);
    assert_eq!(status(&pool, pending.id).await?, "cancelled");

    let running = store
        .enqueue(NewJob::new("cancel.running", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim("worker-a", now, Duration::from_secs(30))
        .await?
        .expect("running job claimed");
    assert!(store.request_cancellation(running.id, now).await?);
    assert!(store.cancellation_requested(running.id).await?);
    assert!(
        !store.complete(running.id, "worker-a", now).await?,
        "completion must not overtake cancellation"
    );
    assert!(store.cancel(running.id, "worker-a", now).await?);
    assert_eq!(status(&pool, running.id).await?, "cancelled");
    assert!(!store.request_cancellation(running.id, now).await?);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_retry_is_bounded_by_attempts_and_has_terminal_failure()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let retried = store
        .enqueue(NewJob::new("retry.test", json!({}), 2).idempotency_key("retry-terminal"))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim("worker-a", now, Duration::from_secs(30))
        .await?
        .expect("first attempt");
    let retry_at = now + TimeDelta::seconds(10);
    assert_eq!(
        store
            .record_failure(retried.id, "worker-a", true, retry_at, "temporary", now,)
            .await?,
        Some(FailureDisposition::Retry)
    );
    assert!(
        store
            .claim("worker-b", now, Duration::from_secs(30))
            .await?
            .is_none(),
        "retry is unavailable before run_after"
    );
    let second = store
        .claim("worker-b", retry_at, Duration::from_secs(30))
        .await?
        .expect("second attempt");
    assert_eq!(second.attempts, 2);
    assert_eq!(
        store
            .record_failure(
                retried.id,
                "worker-b",
                true,
                retry_at + TimeDelta::seconds(20),
                "still temporary",
                retry_at,
            )
            .await?,
        Some(FailureDisposition::Failed)
    );
    assert_eq!(status(&pool, retried.id).await?, "failed");
    let exhausted = store
        .enqueue(
            NewJob::new("retry.test", json!({"ignored": true}), 99)
                .idempotency_key("retry-terminal"),
        )
        .await?
        .job;
    assert_eq!(
        exhausted.failure_reason,
        Some(JobFailureReason::AttemptsExhausted)
    );

    let permanent = store
        .enqueue(NewJob::new("retry.permanent", json!({}), 8).idempotency_key("permanent-terminal"))
        .await?
        .job;
    let permanent_now = Utc::now();
    store
        .claim("worker-c", permanent_now, Duration::from_secs(30))
        .await?
        .expect("permanent job claim");
    assert_eq!(
        store
            .record_failure(
                permanent.id,
                "worker-c",
                false,
                permanent_now,
                "invalid payload",
                permanent_now,
            )
            .await?,
        Some(FailureDisposition::Failed)
    );
    let permanent = store
        .enqueue(
            NewJob::new("retry.permanent", json!({"ignored": true}), 99)
                .idempotency_key("permanent-terminal"),
        )
        .await?
        .job;
    assert_eq!(permanent.failure_reason, Some(JobFailureReason::Permanent));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_completion_is_atomic_lease_owned_and_not_duplicated() -> Result<(), Box<dyn Error>>
{
    let (fixture, pool, store) = fixture().await?;
    let job = store
        .enqueue(NewJob::new("complete.test", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim("worker-a", now, Duration::from_secs(30))
        .await?
        .expect("job claim");

    assert!(!store.complete(job.id, "worker-b", now).await?);
    let mut transaction = pool.begin().await?;
    assert!(
        store
            .complete_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    transaction.rollback().await?;
    assert_eq!(status(&pool, job.id).await?, "running");

    let mut transaction = pool.begin().await?;
    assert!(
        store
            .complete_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    transaction.commit().await?;
    assert_eq!(status(&pool, job.id).await?, "succeeded");
    assert!(!store.complete(job.id, "worker-a", now).await?);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_readiness_executes_the_claim_query_shape() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    store.ready().await?;

    sqlx::query("ALTER TABLE job_outbox RENAME TO unavailable_job_outbox")
        .execute(&pool)
        .await?;
    assert!(
        store.ready().await.is_err(),
        "a reachable database without a compatible outbox is unready"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_terminal_cleanup_uses_independent_cutoffs_and_preserves_active_jobs()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Utc::now();
    let old = now - TimeDelta::days(10);
    let recent = now - TimeDelta::days(1);

    let old_succeeded =
        terminal_job(&store, &pool, "cleanup.succeeded.old", "succeeded", old).await?;
    let recent_succeeded = terminal_job(
        &store,
        &pool,
        "cleanup.succeeded.recent",
        "succeeded",
        recent,
    )
    .await?;
    let old_cancelled =
        terminal_job(&store, &pool, "cleanup.cancelled.old", "cancelled", old).await?;
    let recent_cancelled = terminal_job(
        &store,
        &pool,
        "cleanup.cancelled.recent",
        "cancelled",
        recent,
    )
    .await?;
    let old_failed = terminal_job(&store, &pool, "cleanup.failed.old", "failed", old).await?;
    let recent_failed =
        terminal_job(&store, &pool, "cleanup.failed.recent", "failed", recent).await?;

    let pending = store
        .enqueue(NewJob::new("cleanup.pending", json!({}), 3))
        .await?
        .job;
    let mut expired_job = NewJob::new("cleanup.running", json!({}), 3);
    expired_job.created_at = old;
    expired_job.run_after = old;
    let running = store.enqueue(expired_job).await?.job;
    store
        .claim("cleanup-worker", old, Duration::from_secs(1))
        .await?
        .expect("running job claimed");

    let outcome = store
        .cleanup_terminal_jobs(
            TerminalJobCutoffs {
                succeeded_before: now - TimeDelta::days(5),
                cancelled_before: now - TimeDelta::days(3),
                failed_before: now - TimeDelta::days(7),
            },
            10,
        )
        .await?;

    assert_eq!(
        outcome,
        TerminalJobCleanupOutcome {
            succeeded: 1,
            cancelled: 1,
            failed: 1,
        }
    );
    assert_eq!(outcome.total(), 3);
    for deleted in [old_succeeded, old_cancelled, old_failed] {
        assert!(!job_exists(&pool, deleted).await?);
    }
    for surviving in [
        recent_succeeded,
        recent_cancelled,
        recent_failed,
        pending.id,
        running.id,
    ] {
        assert!(job_exists(&pool, surviving).await?);
    }
    assert_eq!(status(&pool, pending.id).await?, "pending");
    assert_eq!(status(&pool, running.id).await?, "running");

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_terminal_cleanup_is_one_bounded_batch_and_repeated_calls_converge()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Utc::now();
    for sequence in 0..5 {
        terminal_job(
            &store,
            &pool,
            &format!("cleanup.batch.{sequence}"),
            "succeeded",
            now - TimeDelta::days(1),
        )
        .await?;
    }
    let cutoffs = TerminalJobCutoffs {
        succeeded_before: now,
        cancelled_before: now,
        failed_before: now,
    };

    let first = store.cleanup_terminal_jobs(cutoffs, 2).await?;
    let second = store.cleanup_terminal_jobs(cutoffs, 2).await?;
    let third = store.cleanup_terminal_jobs(cutoffs, 2).await?;
    let empty = store.cleanup_terminal_jobs(cutoffs, 2).await?;

    assert_eq!([first.total(), second.total(), third.total()], [2, 2, 1]);
    assert_eq!(empty, TerminalJobCleanupOutcome::default());
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0, "returned counts match committed deletes");
    assert!(matches!(
        store.cleanup_terminal_jobs(cutoffs, 0).await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store
            .cleanup_terminal_jobs(cutoffs, MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE + 1)
            .await,
        Err(StoreError::InvalidInput(_))
    ));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_terminal_cleanup_skips_concurrent_claim_and_completion()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Utc::now();
    let pending = store
        .enqueue(NewJob::new("cleanup.concurrent-claim", json!({}), 3))
        .await?
        .job;
    let claim_now = Utc::now();
    let mut claim_transaction = pool.begin().await?;
    sqlx::query(
        "UPDATE job_outbox SET status = 'running', attempts = 1, locked_by = 'claim-worker', locked_until = $2, updated_at = $3 WHERE id = $1",
    )
    .bind(pending.id)
    .bind(claim_now + TimeDelta::minutes(1))
    .bind(claim_now)
    .execute(&mut *claim_transaction)
    .await?;

    let cutoffs = TerminalJobCutoffs {
        succeeded_before: now + TimeDelta::days(1),
        cancelled_before: now + TimeDelta::days(1),
        failed_before: now + TimeDelta::days(1),
    };
    assert_eq!(
        store.cleanup_terminal_jobs(cutoffs, 10).await?,
        TerminalJobCleanupOutcome::default()
    );
    claim_transaction.commit().await?;
    assert_eq!(status(&pool, pending.id).await?, "running");

    let completing = store
        .enqueue(NewJob::new("cleanup.concurrent-completion", json!({}), 3))
        .await?
        .job;
    let completion_now = Utc::now();
    store
        .claim("complete-worker", completion_now, Duration::from_secs(60))
        .await?
        .expect("job claimed for completion");
    let mut completion_transaction = pool.begin().await?;
    assert!(
        store
            .complete_in_transaction(
                &mut completion_transaction,
                completing.id,
                "complete-worker",
                completion_now,
            )
            .await?
    );
    assert_eq!(
        store.cleanup_terminal_jobs(cutoffs, 10).await?,
        TerminalJobCleanupOutcome::default()
    );
    completion_transaction.commit().await?;
    assert_eq!(status(&pool, completing.id).await?, "succeeded");

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_recurring_slot_enqueue_is_restart_safe_and_failure_keeps_current_running()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Utc::now();
    let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60))?;
    let current_slot = interval.slot_at(now)?;
    let next_slot = interval.next_slot(current_slot, now)?;
    let current = store
        .enqueue(
            NewJob::new(
                "recurring.test",
                json!({"slot": current_slot.starts_at()}),
                3,
            )
            .run_after(current_slot.starts_at())
            .idempotency_key(current_slot.identifier()),
        )
        .await?
        .job;
    let claim_now = Utc::now();
    store
        .claim("recurring-worker", claim_now, Duration::from_secs(60))
        .await?
        .expect("current recurring job claimed");

    let next_job = || {
        NewJob::new("recurring.test", json!({"slot": next_slot.starts_at()}), 3)
            .run_after(next_slot.starts_at())
            .idempotency_key(next_slot.identifier())
    };
    assert!(store.enqueue(next_job()).await?.created);
    assert!(
        !store.enqueue(next_job()).await?.created,
        "duplicate delivery or restart reuses the next slot"
    );

    let enqueue_error = store
        .enqueue(
            NewJob::new("recurring.invalid", json!({}), 0)
                .run_after(next_slot.starts_at())
                .idempotency_key(next_slot.identifier()),
        )
        .await;
    assert!(matches!(enqueue_error, Err(StoreError::InvalidInput(_))));
    assert_eq!(
        status(&pool, current.id).await?,
        "running",
        "the handler must return an error after next-slot enqueue fails"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_v051_schema_upgrades_to_failure_reasons() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(POSTGRES_MIGRATION_SQL).execute(&pool).await?;

    sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, status, attempts, max_attempts) VALUES ($1, 'legacy.exhausted', '{}', 'failed', 3, 3), ($2, 'legacy.permanent', '{}', 'failed', 1, 3)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(uuid::Uuid::new_v4())
    .execute(&pool)
    .await?;

    sqlx::raw_sql(POSTGRES_MIGRATION_0002_SQL)
        .execute(&pool)
        .await?;

    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT failure_reason FROM job_outbox ORDER BY job_type")
            .fetch_all(&pool)
            .await?;
    assert_eq!(reasons, ["attempts_exhausted", "permanent"]);

    let store = PostgresJobStore::new(pool.clone());
    let current = store
        .enqueue(NewJob::new("current.failure", json!({}), 2).idempotency_key("upgrade-check"))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim("upgrade-worker", now, Duration::from_secs(30))
        .await?
        .expect("current job is claimable after the upgrade");
    assert_eq!(
        store
            .record_failure(
                current.id,
                "upgrade-worker",
                false,
                now,
                "permanent failure",
                now,
            )
            .await?,
        Some(FailureDisposition::Failed)
    );
    let current = store
        .enqueue(NewJob::new("current.failure", json!({}), 99).idempotency_key("upgrade-check"))
        .await?
        .job;
    assert_eq!(current.failure_reason, Some(JobFailureReason::Permanent));

    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}

async fn status(pool: &PgPool, job_id: uuid::Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn failure_reason(pool: &PgPool, job_id: uuid::Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT failure_reason FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn terminal_job(
    store: &PostgresJobStore,
    pool: &PgPool,
    job_type: &str,
    terminal_status: &str,
    updated_at: chrono::DateTime<Utc>,
) -> Result<uuid::Uuid, Box<dyn Error>> {
    let mut new_job = NewJob::new(job_type, json!({}), 3);
    new_job.created_at = updated_at;
    new_job.run_after = updated_at;
    let job = store.enqueue(new_job).await?.job;
    let failure_reason = (terminal_status == "failed").then_some("permanent");
    sqlx::query(
        "UPDATE job_outbox SET status = $2, failure_reason = $3, updated_at = $4 WHERE id = $1",
    )
    .bind(job.id)
    .bind(terminal_status)
    .bind(failure_reason)
    .bind(updated_at)
    .execute(pool)
    .await?;
    Ok(job.id)
}

async fn job_exists(pool: &PgPool, job_id: uuid::Uuid) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM job_outbox WHERE id = $1)")
        .bind(job_id)
        .fetch_one(pool)
        .await
}
