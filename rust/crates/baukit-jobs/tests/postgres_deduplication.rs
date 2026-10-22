use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{
    FailureDisposition, JobStatus, JobStore as _, NewJob, POSTGRES_MIGRATION_0002_SQL,
    POSTGRES_MIGRATION_0003_SQL, POSTGRES_MIGRATION_0004_SQL, POSTGRES_MIGRATION_SQL,
    PostgresJobStore,
};
use chrono::{TimeDelta, Utc};
use serde_json::json;
use sqlx::PgPool;
use tokio::sync::Barrier;

const WAIT_LIMIT: Duration = Duration::from_secs(30);
const LEASE: Duration = Duration::from_secs(60);
const KEY: &str = "resource:42";
const KIND: &str = "cache.refresh";

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn terminal_jobs_release_the_key_and_running_jobs_keep_it() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    for terminal_status in [
        JobStatus::Succeeded,
        JobStatus::Failed,
        JobStatus::Cancelled,
    ] {
        let original = store.enqueue_deduplicated(new_job(), KEY).await?;
        assert!(original.created);
        let now = Utc::now();
        let claimed = store
            .claim("worker", &[KIND], now, LEASE)
            .await?
            .expect("job claimed");
        assert_eq!(claimed.id, original.job.id);
        let duplicate = store
            .enqueue_deduplicated(NewJob::new(KIND, json!({"ignored": true}), 99), KEY)
            .await?;
        assert!(!duplicate.created);
        assert_eq!(
            duplicate.job, claimed,
            "a duplicate does not update the row"
        );

        match terminal_status {
            JobStatus::Succeeded => {
                let mut transaction = pool.begin().await?;
                assert!(
                    store
                        .complete_in_transaction(&mut transaction, claimed.id, "worker", now)
                        .await?
                );
                transaction.commit().await?;
            }
            JobStatus::Failed => assert_eq!(
                store
                    .record_failure(claimed.id, "worker", false, now, "permanent", now)
                    .await?,
                Some(FailureDisposition::Failed)
            ),
            JobStatus::Cancelled => {
                assert!(store.request_cancellation(claimed.id, now).await?);
                let duplicate = store.enqueue_deduplicated(new_job(), KEY).await?;
                assert!(!duplicate.created, "a running cancellation is still active");
                assert_eq!(duplicate.job.id, claimed.id);
                assert!(store.cancel(claimed.id, "worker", now).await?);
            }
            other => panic!("unexpected test status {other:?}"),
        }
        let replacement = store.enqueue_deduplicated(new_job(), KEY).await?;
        assert!(replacement.created);
        assert_ne!(replacement.job.id, original.job.id);
        assert!(
            store
                .request_cancellation(replacement.job.id, Utc::now())
                .await?
        );
        let stored_key: String =
            sqlx::query_scalar("SELECT dedupe_key FROM job_outbox WHERE id = $1")
                .bind(original.job.id)
                .fetch_one(&pool)
                .await?;
        assert_eq!(
            stored_key, KEY,
            "terminal jobs retain the key for inspection"
        );
    }
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn pending_retries_and_expired_leases_keep_the_key_until_terminal()
-> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    let scheduled = Utc::now() + TimeDelta::hours(1);
    let original = store
        .enqueue_deduplicated(new_job().run_after(scheduled), KEY)
        .await?;
    let duplicate = store.enqueue_deduplicated(new_job(), KEY).await?;
    assert!(!duplicate.created);
    assert_eq!(duplicate.job, original.job);
    let first = store
        .claim("worker-a", &[KIND], scheduled, LEASE)
        .await?
        .expect("scheduled job claimed");
    let retry_at = scheduled + TimeDelta::minutes(1);
    assert_eq!(
        store
            .record_failure(first.id, "worker-a", true, retry_at, "temporary", scheduled)
            .await?,
        Some(FailureDisposition::Retry)
    );
    let duplicate = store.enqueue_deduplicated(new_job(), KEY).await?;
    assert!(!duplicate.created);
    assert_eq!(duplicate.job.status, JobStatus::Pending);
    assert_eq!(
        duplicate.job.run_after.timestamp_micros(),
        retry_at.timestamp_micros()
    );
    assert_eq!(duplicate.job.attempts, 1);
    store
        .claim("worker-b", &[KIND], retry_at, Duration::from_secs(1))
        .await?
        .expect("retry claimed");
    assert!(!store.enqueue_deduplicated(new_job(), KEY).await?.created);
    let reclaimed_at = retry_at + TimeDelta::seconds(2);
    let reclaimed = store
        .claim("worker-c", &[KIND], reclaimed_at, Duration::from_secs(1))
        .await?
        .expect("expired lease reclaimed");
    assert_eq!(reclaimed.id, first.id);
    assert_eq!(reclaimed.attempts, 3);
    assert!(!store.enqueue_deduplicated(new_job(), KEY).await?.created);
    let expired_at = reclaimed_at + TimeDelta::seconds(2);
    assert!(
        store
            .claim("worker-d", &[KIND], expired_at, LEASE)
            .await?
            .is_none()
    );
    let replacement = store.enqueue_deduplicated(new_job(), KEY).await?;
    assert!(
        replacement.created,
        "exhausted lease recovery releases the key"
    );
    assert!(
        store
            .request_cancellation(replacement.job.id, Utc::now())
            .await?
    );
    let pending_replacement = store.enqueue_deduplicated(new_job(), KEY).await?;
    assert!(
        pending_replacement.created,
        "pending cancellation releases the key"
    );
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_enqueues_insert_one_active_row() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    const PRODUCERS: usize = 8;
    let barrier = Arc::new(Barrier::new(PRODUCERS + 1));
    let mut tasks = Vec::new();
    for producer in 0..PRODUCERS {
        let store = store.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .enqueue_deduplicated(NewJob::new(KIND, json!({"producer": producer}), 3), KEY)
                .await
        }));
    }
    barrier.wait().await;
    tokio::time::timeout(WAIT_LIMIT, async {
        let mut created = 0;
        let mut original = None;
        for task in tasks {
            let outcome = task.await??;
            created += usize::from(outcome.created);
            if let Some(original) = &original {
                assert_eq!(&outcome.job, original);
            } else {
                original = Some(outcome.job);
            }
        }
        assert_eq!(created, 1);
        Ok::<(), Box<dyn Error>>(())
    })
    .await??;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM job_outbox")
        .fetch_one(&pool)
        .await?;
    assert_eq!(count, 1);
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn uncommitted_enqueues_serialize_and_rollback_releases_the_key() -> Result<(), Box<dyn Error>>
{
    let (_fixture, pool, store) = fixture().await?;
    for commit in [true, false] {
        let key = format!("{KEY}:{commit}");
        let mut transaction = pool.begin().await?;
        let original = store
            .enqueue_deduplicated_in_transaction(&mut transaction, new_job(), &key)
            .await?;
        let visible: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM job_outbox WHERE id = $1)")
                .bind(original.job.id)
                .fetch_one(&pool)
                .await?;
        assert!(!visible, "caller-owned transaction has not committed");
        let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&mut *transaction)
            .await?;
        let concurrent_store = store.clone();
        let concurrent_key = key.clone();
        let concurrent = tokio::spawn(async move {
            concurrent_store
                .enqueue_deduplicated(new_job(), &concurrent_key)
                .await
        });
        wait_for_blocked_query(&pool, blocker).await?;
        assert!(!concurrent.is_finished());
        if commit {
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
        }
        let result = tokio::time::timeout(WAIT_LIMIT, concurrent).await???;
        assert_eq!(result.created, !commit);
        if commit {
            assert_eq!(result.job, original.job);
        } else {
            assert_ne!(result.job.id, original.job.id);
        }
    }
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn returning_an_active_job_holds_its_lock_until_transaction_end() -> Result<(), Box<dyn Error>>
{
    let (_fixture, pool, store) = fixture().await?;
    let original = store.enqueue_deduplicated(new_job(), KEY).await?;
    let now = Utc::now();
    store
        .claim("worker", &[KIND], now, LEASE)
        .await?
        .expect("claimed");
    let mut transaction = pool.begin().await?;
    let duplicate = store
        .enqueue_deduplicated_in_transaction(&mut transaction, new_job(), KEY)
        .await?;
    assert!(!duplicate.created);
    assert_eq!(duplicate.job.id, original.job.id);
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await?;
    let completion_store = store.clone();
    let completion = tokio::spawn(async move {
        completion_store
            .complete(original.job.id, "worker", now)
            .await
    });
    wait_for_blocked_query(&pool, blocker).await?;
    assert!(!completion.is_finished());
    transaction.commit().await?;
    assert!(tokio::time::timeout(WAIT_LIMIT, completion).await???);
    assert!(store.enqueue_deduplicated(new_job(), KEY).await?.created);
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn completion_between_insert_and_lookup_retries_with_a_new_job() -> Result<(), Box<dyn Error>>
{
    let (_fixture, pool, store) = fixture().await?;
    let original = store.enqueue_deduplicated(new_job(), KEY).await?;
    let now = Utc::now();
    store
        .claim("worker", &[KIND], now, LEASE)
        .await?
        .expect("claimed");
    let mut transaction = pool.begin().await?;
    assert!(
        store
            .check_lease_in_transaction(&mut transaction, original.job.id, "worker", now)
            .await?
    );
    let blocker: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await?;
    let concurrent_store = store.clone();
    let concurrent =
        tokio::spawn(async move { concurrent_store.enqueue_deduplicated(new_job(), KEY).await });
    wait_for_blocked_query(&pool, blocker).await?;
    let query: String = sqlx::query_scalar(
        "SELECT query FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock'",
    ).bind(blocker).fetch_one(&pool).await?;
    assert!(
        query.starts_with("SELECT"),
        "enqueue reached the lookup: {query}"
    );
    assert!(
        store
            .complete_in_transaction(&mut transaction, original.job.id, "worker", now)
            .await?
    );
    transaction.commit().await?;
    let replacement = tokio::time::timeout(WAIT_LIMIT, concurrent).await???;
    assert!(replacement.created, "the conflicting job is now terminal");
    assert_ne!(replacement.job.id, original.job.id);
    pool.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn forward_migration_preserves_legacy_keys_and_keeps_modes_independent()
-> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    for migration in [
        POSTGRES_MIGRATION_SQL,
        POSTGRES_MIGRATION_0002_SQL,
        POSTGRES_MIGRATION_0003_SQL,
    ] {
        sqlx::raw_sql(migration).execute(&pool).await?;
    }
    let store = PostgresJobStore::new(pool.clone());
    let legacy = store.enqueue(new_job().idempotency_key(KEY)).await?;
    let before: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(job_outbox) FROM job_outbox WHERE id = $1")
            .bind(legacy.job.id)
            .fetch_one(&pool)
            .await?;
    sqlx::raw_sql(POSTGRES_MIGRATION_0004_SQL)
        .execute(&pool)
        .await?;
    let mut after: serde_json::Value =
        sqlx::query_scalar("SELECT to_jsonb(job_outbox) FROM job_outbox WHERE id = $1")
            .bind(legacy.job.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(
        after
            .as_object_mut()
            .expect("row object")
            .remove("dedupe_key"),
        Some(serde_json::Value::Null)
    );
    assert_eq!(before, after);
    assert!(
        store
            .request_cancellation(legacy.job.id, Utc::now())
            .await?
    );
    let duplicate = store.enqueue(new_job().idempotency_key(KEY)).await?;
    assert!(!duplicate.created, "legacy idempotency is permanent");
    assert_eq!(duplicate.job.id, legacy.job.id);
    let active = store.enqueue_deduplicated(new_job(), KEY).await?;
    assert!(active.created, "the key modes are independent");
    assert!(active.job.idempotency_key.is_none());
    let other_type = store
        .enqueue_deduplicated(NewJob::new("other.refresh", json!({}), 3), KEY)
        .await?;
    assert!(other_type.created, "keys are scoped by type");
    let unkeyed = store.enqueue(new_job()).await?;
    assert!(unkeyed.created, "unkeyed jobs do not occupy a dedupe key");
    pool.close().await;
    Ok(())
}

fn new_job() -> NewJob {
    NewJob::new(KIND, json!({"resource_id": 42}), 3)
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}

async fn wait_for_blocked_query(pool: &PgPool, blocker: i32) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(WAIT_LIMIT, async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock')",
            )
            .bind(blocker)
            .fetch_one(pool)
            .await?;
            if blocked {
                return Ok::<(), sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}
