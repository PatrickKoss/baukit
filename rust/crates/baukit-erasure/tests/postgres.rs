use baukit_config::Secret;
use baukit_erasure::{
    ErasureError, ErasureFuture, ErasureService, ErasureState, IdentityDeletionError,
    IdentityDeletionHandler, POSTGRES_MIGRATION_SQL, PostgresErasureStore, ProductErasure,
};
use baukit_jobs::{JobStore, PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_runtime::ShutdownToken;
use baukit_test::{FakeIdentityAccountDeleter, PostgresTestContainer};
use sqlx::{PgConnection, PgPool};
use std::{error::Error, sync::Arc, time::Duration};

struct Product;
impl ProductErasure for Product {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM profiles WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}
async fn fixture() -> Result<
    (
        PostgresTestContainer,
        PgPool,
        PostgresErasureStore,
        Arc<FakeIdentityAccountDeleter>,
        ErasureService,
    ),
    Box<dyn Error>,
> {
    let container = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(container.connection_url()).await?;
    for migration in [
        baukit_jobs::POSTGRES_MIGRATION_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
        POSTGRES_MIGRATION_SQL,
        "CREATE TABLE profiles (subject TEXT PRIMARY KEY); INSERT INTO profiles VALUES ('alice'), ('bob');",
    ] {
        sqlx::raw_sql(migration).execute(&pool).await?;
    }
    let store = PostgresErasureStore::new(
        pool.clone(),
        Secret::new("test-key-with-at-least-32-bytes-of-entropy".into()),
    )?;
    let fake = Arc::new(FakeIdentityAccountDeleter::default());
    let service = ErasureService::new(
        store.clone(),
        fake.clone(),
        "test".into(),
        Duration::from_secs(1),
        2,
    )?;
    Ok((container, pool, store, fake, service))
}
async fn count(pool: &PgPool, table: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(match table {
        "profiles" => "SELECT count(*) FROM profiles",
        "erasure_operations" => "SELECT count(*) FROM erasure_operations",
        "erasure_fences" => "SELECT count(*) FROM erasure_fences",
        "job_outbox" => "SELECT count(*) FROM job_outbox",
        _ => panic!("unknown test table"),
    })
    .fetch_one(pool)
    .await
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn atomic_rollback_after_product_callback() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store, fake, service) = fixture().await?;
    sqlx::raw_sql("CREATE FUNCTION reject_enqueue() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected failure'; END; $$; CREATE TRIGGER reject_enqueue BEFORE INSERT ON job_outbox FOR EACH ROW EXECUTE FUNCTION reject_enqueue();").execute(&pool).await?;
    assert!(
        service
            .erase("alice", "atomic-key-0000001", &Product)
            .await
            .is_err()
    );
    assert_eq!(count(&pool, "profiles").await?, 2);
    assert_eq!(count(&pool, "erasure_operations").await?, 0);
    assert_eq!(count(&pool, "erasure_fences").await?, 0);
    assert_eq!(count(&pool, "job_outbox").await?, 0);
    assert!(!store.is_fenced("alice").await?);
    assert!(fake.calls().is_empty());
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn completed_replay_conflict_and_fence() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store, fake, service) = fixture().await?;
    let first = service
        .erase("alice", "replay-key-0000001", &Product)
        .await?;
    assert_eq!(first.status, ErasureState::Completed);
    assert_eq!(
        service
            .erase("alice", "replay-key-0000001", &Product)
            .await?,
        first
    );
    assert!(matches!(
        service.erase("bob", "replay-key-0000001", &Product).await,
        Err(ErasureError::Conflict)
    ));
    assert!(matches!(
        service.erase("alice", "different-key-0001", &Product).await,
        Err(ErasureError::ProfileErased)
    ));
    assert!(store.status("bob", first.operation_id).await?.is_none());
    assert_eq!(
        store.status("alice", first.operation_id).await?,
        Some(first)
    );
    let mut transaction = pool.begin().await?;
    assert!(matches!(
        store.guard_subject(&mut transaction, "alice").await,
        Err(ErasureError::ProfileErased)
    ));
    assert_eq!(count(&pool, "profiles").await?, 1);
    assert_eq!(count(&pool, "job_outbox").await?, 0);
    assert_eq!(fake.calls(), ["alice"]);
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn concurrent_same_key_only_erases_once() -> Result<(), Box<dyn Error>> {
    let (_container, pool, _store, fake, service) = fixture().await?;
    let barrier = Arc::new(tokio::sync::Barrier::new(9));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let service = service.clone();
        let barrier = barrier.clone();
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            service.erase("alice", "concurrent-key-001", &Product).await
        }));
    }
    barrier.wait().await;
    let mut operation = None;
    for task in tasks {
        let outcome = task.await??;
        if let Some(id) = operation {
            assert_eq!(outcome.operation_id, id);
        }
        operation = Some(outcome.operation_id);
    }
    assert_eq!(count(&pool, "erasure_operations").await?, 1);
    assert_eq!(count(&pool, "erasure_fences").await?, 1);
    assert_eq!(fake.calls(), ["alice"]);
    Ok(())
}
async fn run_until(
    store: PostgresErasureStore,
    fake: Arc<FakeIdentityAccountDeleter>,
    subject: &str,
    operation: uuid::Uuid,
    expected: ErasureState,
) -> Result<(), Box<dyn Error>> {
    let runner = WorkerRunner::new(
        Arc::new(PostgresJobStore::new(store.pool().clone())),
        Arc::new(IdentityDeletionHandler::new(
            store.clone(),
            "test".into(),
            fake,
        )),
        WorkerConfig {
            poll_interval: Duration::from_millis(10),
            retry_initial: Duration::from_millis(10),
            retry_max: Duration::from_millis(10),
            ..WorkerConfig::default()
        },
    )?;
    let shutdown = ShutdownToken::new(Duration::from_secs(1));
    let worker = tokio::spawn(runner.run(shutdown.clone()));
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if store
                .status(subject, operation)
                .await?
                .is_some_and(|receipt| receipt.status == expected)
            {
                break Ok::<_, ErasureError>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    shutdown.trigger();
    worker.await??;
    result??;
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn worker_retries_completes_and_removes_subject_payload() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store, fake, service) = fixture().await?;
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    let pending = service
        .erase("alice", "worker-retry-key-01", &Product)
        .await?;
    assert_eq!(pending.status, ErasureState::Pending);
    assert_eq!(count(&pool, "profiles").await?, 1);
    assert_eq!(count(&pool, "job_outbox").await?, 1);
    assert!(store.is_fenced("alice").await?);
    assert_eq!(
        service
            .erase("alice", "worker-retry-key-01", &Product)
            .await?,
        pending
    );
    run_until(
        store.clone(),
        fake.clone(),
        "alice",
        pending.operation_id,
        ErasureState::Completed,
    )
    .await?;
    assert_eq!(count(&pool, "job_outbox").await?, 0);
    assert_eq!(fake.calls(), ["alice", "alice", "alice"]);
    let raw: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM erasure_operations WHERE response::text LIKE '%alice%'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(raw, 0);
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn permanent_and_exhausted_failures_remain_rerunnable() -> Result<(), Box<dyn Error>> {
    for failure in [
        IdentityDeletionError::Permanent,
        IdentityDeletionError::Retryable,
    ] {
        let (_container, pool, store, fake, service) = fixture().await?;
        for _ in 0..3 {
            fake.push_outcome(Err(failure));
        }
        let pending = service
            .erase("alice", "failed-worker-key-01", &Product)
            .await?;
        run_until(
            store.clone(),
            fake.clone(),
            "alice",
            pending.operation_id,
            ErasureState::Failed,
        )
        .await?;
        assert!(store.is_fenced("alice").await?);
        let state: String = sqlx::query_scalar("SELECT status FROM job_outbox")
            .fetch_one(&pool)
            .await?;
        assert_eq!(state, "failed");
        if failure == IdentityDeletionError::Permanent {
            assert_eq!(fake.calls().len(), 2);
        } else {
            assert_eq!(fake.calls().len(), 3);
        }
        let restored_fake = Arc::new(FakeIdentityAccountDeleter::default());
        sqlx::query("UPDATE job_outbox SET status = 'pending', attempts = 0, failure_reason = NULL, run_after = now()").execute(&pool).await?;
        run_until(
            store,
            restored_fake.clone(),
            "alice",
            pending.operation_id,
            ErasureState::Completed,
        )
        .await?;
        assert_eq!(count(&pool, "job_outbox").await?, 0);
        assert_eq!(restored_fake.calls(), ["alice"]);
    }
    Ok(())
}
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn final_expired_lease_fails_operation() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store, fake, service) = fixture().await?;
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    let pending = service
        .erase("alice", "expired-lease-key-01", &Product)
        .await?;
    sqlx::query("UPDATE job_outbox SET status = 'running', attempts = max_attempts, locked_by = 'dead-worker', locked_until = now() - interval '1 second'").execute(&pool).await?;
    PostgresJobStore::new(pool)
        .claim(
            "replacement",
            &[baukit_erasure::IDENTITY_DELETE_JOB_TYPE],
            chrono::Utc::now(),
            Duration::from_secs(5),
        )
        .await?;
    assert_eq!(
        store
            .status("alice", pending.operation_id)
            .await?
            .expect("receipt")
            .status,
        ErasureState::Failed
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn inline_completion_database_failure_returns_durable_acceptance()
-> Result<(), Box<dyn Error>> {
    let (_container, pool, store, fake, service) = fixture().await?;
    sqlx::raw_sql("CREATE FUNCTION reject_completion() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected receipt failure'; END; $$; CREATE TRIGGER reject_completion BEFORE UPDATE ON erasure_operations FOR EACH ROW EXECUTE FUNCTION reject_completion();").execute(&pool).await?;
    let pending = service
        .erase("alice", "inline-db-failure-01", &Product)
        .await?;
    assert_eq!(pending.status, ErasureState::Pending);
    assert_eq!(count(&pool, "profiles").await?, 1);
    assert_eq!(count(&pool, "job_outbox").await?, 1);
    assert!(store.is_fenced("alice").await?);
    assert_eq!(fake.calls(), ["alice"]);
    sqlx::query("DROP TRIGGER reject_completion ON erasure_operations")
        .execute(&pool)
        .await?;
    run_until(
        store,
        fake.clone(),
        "alice",
        pending.operation_id,
        ErasureState::Completed,
    )
    .await?;
    assert_eq!(count(&pool, "job_outbox").await?, 0);
    assert_eq!(fake.calls(), ["alice", "alice"]);
    Ok(())
}
