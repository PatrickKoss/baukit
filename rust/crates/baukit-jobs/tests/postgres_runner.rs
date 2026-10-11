use std::{error::Error, future::pending, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{
    ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler, JobStore as _, NewJob,
    PostgresJobStore, WorkerConfig, WorkerRunner,
};
use baukit_runtime::ShutdownToken;
use chrono::Utc;
use serde_json::{Value, json};
use sqlx::{PgPool, Row as _, postgres::PgPoolOptions};
use tokio::sync::{Notify, mpsc};

const WAIT_LIMIT: Duration = Duration::from_secs(15);
const POLL_INTERVAL: Duration = Duration::from_millis(10);
const SHORT_TIMEOUT: Duration = Duration::from_secs(1);
const LONG_TIMEOUT: Duration = Duration::from_secs(60);
const ORIGINAL_PAYLOAD: Value = Value::Null;

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn cancellation_drops_the_handler_transaction_before_recording_the_outcome()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new().await?;
    let cancellation_store = Arc::new(PostgresJobStore::new(fixture.pool.clone()));
    let job = fixture
        .store
        .enqueue(NewJob::new("locked.job", ORIGINAL_PAYLOAD, 3))
        .await?
        .job;
    let (handler, mut locked) = fixture.handler();
    let runner = fixture.runner(handler.clone(), LONG_TIMEOUT)?;
    let task = tokio::spawn(runner.run(fixture.shutdown.clone()));

    tokio::time::timeout(WAIT_LIMIT, async {
        handler.started.notified().await;
        let polling_connection = fixture.store_pool.acquire().await?;
        // A new cancellation UPDATE waits for FOR UPDATE. Commit the durable
        // request first, then repeat it while the handler holds the lock.
        assert!(
            cancellation_store
                .request_cancellation(job.id, Utc::now())
                .await?
        );
        handler.lock_allowed.notify_one();
        let handler_pid = locked.recv().await.expect("handler holds the row lock");
        assert_idle_transaction(&fixture.pool, handler_pid).await?;

        let requesting_store = Arc::clone(&cancellation_store);
        let request = tokio::spawn(async move {
            requesting_store
                .request_cancellation(job.id, Utc::now())
                .await
        });
        wait_for_blocked_request(&fixture.pool, handler_pid).await?;
        assert!(
            !request.is_finished(),
            "cancellation UPDATE waits for the handler"
        );
        assert!(
            cancellation_store.cancellation_requested(job.id).await?,
            "the cancellation SELECT reads the committed request despite the row lock"
        );
        drop(polling_connection);

        task.await??;
        request.await??;
        assert_outcome(&fixture.pool, job.id, "cancelled", None).await?;
        Ok::<(), Box<dyn Error>>(())
    })
    .await
    .expect("cancellation must release the handler transaction within 15 seconds")?;

    fixture.close().await;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn timeout_drops_the_handler_transaction_before_recording_the_outcome()
-> Result<(), Box<dyn Error>> {
    let fixture = Fixture::new().await?;
    let job = fixture
        .store
        .enqueue(NewJob::new("locked.job", ORIGINAL_PAYLOAD, 3))
        .await?
        .job;
    let (handler, mut locked) = fixture.handler();
    handler.lock_allowed.notify_one();
    let runner = fixture.runner(handler, SHORT_TIMEOUT)?;
    let task = tokio::spawn(runner.run(fixture.shutdown.clone()));

    tokio::time::timeout(WAIT_LIMIT, async {
        let handler_pid = locked.recv().await.expect("handler holds the row lock");
        assert_idle_transaction(&fixture.pool, handler_pid).await?;
        assert!(
            !fixture.store.cancellation_requested(job.id).await?,
            "the cancellation SELECT completes despite the row lock"
        );

        task.await??;
        assert_outcome(&fixture.pool, job.id, "pending", Some("job timed out")).await?;
        Ok::<(), Box<dyn Error>>(())
    })
    .await
    .expect("timeout must release the handler transaction within 15 seconds")?;

    fixture.close().await;
    Ok(())
}

struct LockingHandler {
    pool: PgPool,
    shutdown: ShutdownToken,
    started: Notify,
    lock_allowed: Notify,
    locked: mpsc::Sender<i32>,
}

impl JobHandler for LockingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &["locked.job"]
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            self.started.notify_one();
            self.lock_allowed.notified().await;
            let mut transaction = self.pool.begin().await.expect("handler transaction begins");
            let pid: i32 = sqlx::query_scalar(
                "SELECT pg_backend_pid() FROM job_outbox WHERE id = $1 FOR UPDATE",
            )
            .bind(job.id)
            .fetch_one(&mut *transaction)
            .await
            .expect("handler locks its claimed row");
            sqlx::query("UPDATE job_outbox SET payload = $2 WHERE id = $1")
                .bind(job.id)
                .bind(json!({"uncommitted": true}))
                .execute(&mut *transaction)
                .await
                .expect("handler writes inside the transaction");
            self.locked
                .send(pid)
                .await
                .expect("test receives the lock signal");
            self.shutdown.trigger();
            pending::<()>().await;
            drop(transaction);
            Ok(())
        })
    }
}

struct Fixture {
    container: baukit_test::PostgresTestContainer,
    pool: PgPool,
    store_pool: PgPool,
    handler_pool: PgPool,
    store: Arc<PostgresJobStore>,
    shutdown: ShutdownToken,
}

impl Fixture {
    async fn new() -> Result<Self, Box<dyn Error>> {
        let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
        let container = baukit_test::start_postgres_with_migrations(migrations).await?;
        let pool = PgPool::connect(container.connection_url()).await?;
        let store_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(container.connection_url())
            .await?;
        let handler_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(container.connection_url())
            .await?;
        Ok(Self {
            container,
            pool,
            store: Arc::new(PostgresJobStore::new(store_pool.clone())),
            store_pool,
            handler_pool,
            shutdown: ShutdownToken::new(WAIT_LIMIT),
        })
    }

    fn handler(&self) -> (Arc<LockingHandler>, mpsc::Receiver<i32>) {
        let (locked, receiver) = mpsc::channel(1);
        (
            Arc::new(LockingHandler {
                pool: self.handler_pool.clone(),
                shutdown: self.shutdown.clone(),
                started: Notify::new(),
                lock_allowed: Notify::new(),
                locked,
            }),
            receiver,
        )
    }

    fn runner(
        &self,
        handler: Arc<LockingHandler>,
        job_timeout: Duration,
    ) -> Result<WorkerRunner, baukit_jobs::RunnerError> {
        WorkerRunner::new(
            self.store.clone(),
            handler,
            WorkerConfig {
                worker_id: "locked-job-worker".to_owned(),
                concurrency: 1,
                poll_interval: POLL_INTERVAL,
                lease_duration: Duration::from_secs(120),
                job_timeout,
                cancellation_poll_interval: POLL_INTERVAL,
                ..WorkerConfig::default()
            },
        )
    }

    async fn close(self) {
        self.handler_pool.close().await;
        self.store_pool.close().await;
        self.pool.close().await;
        drop(self.container);
    }
}

async fn assert_idle_transaction(pool: &PgPool, handler_pid: i32) -> Result<(), sqlx::Error> {
    let state: String = sqlx::query_scalar("SELECT state FROM pg_stat_activity WHERE pid = $1")
        .bind(handler_pid)
        .fetch_one(pool)
        .await?;
    assert_eq!(state, "idle in transaction");
    Ok(())
}

async fn wait_for_blocked_request(pool: &PgPool, handler_pid: i32) -> Result<(), sqlx::Error> {
    loop {
        let blocked: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND $1 = ANY(pg_blocking_pids(pid)))",
        )
        .bind(handler_pid)
        .fetch_one(pool)
        .await?;
        if blocked {
            return Ok(());
        }
        tokio::task::yield_now().await;
    }
}

async fn assert_outcome(
    pool: &PgPool,
    job_id: uuid::Uuid,
    expected_status: &str,
    expected_error: Option<&str>,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query(
        "SELECT status, attempts, locked_by, locked_until, cancel_requested_at, last_error, payload FROM job_outbox WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await?;
    assert_eq!(row.try_get::<String, _>("status")?, expected_status);
    assert_eq!(row.try_get::<i32, _>("attempts")?, 1);
    assert_eq!(row.try_get::<Option<String>, _>("locked_by")?, None);
    for field in ["locked_until", "cancel_requested_at"] {
        assert_eq!(
            row.try_get::<Option<chrono::DateTime<Utc>>, _>(field)?,
            None
        );
    }
    assert_eq!(
        row.try_get::<Option<String>, _>("last_error")?.as_deref(),
        expected_error
    );
    assert_eq!(row.try_get::<Value, _>("payload")?, ORIGINAL_PAYLOAD);

    let idle_transactions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state = 'idle in transaction'",
    )
    .fetch_one(pool)
    .await?;
    assert_eq!(idle_transactions, 0, "handler transaction was rolled back");
    Ok(())
}
