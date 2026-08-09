use std::{collections::HashSet, error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{FailureDisposition, JobStatus, JobStore as _, NewJob, PostgresJobStore};
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
        .enqueue(NewJob::new("retry.test", json!({}), 2))
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

    let permanent = store
        .enqueue(NewJob::new("retry.permanent", json!({}), 8))
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
