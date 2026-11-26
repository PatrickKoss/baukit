#![cfg(test)]
use std::{
    error::Error,
    num::NonZeroU32,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use baukit_jobs::{
    CLAIM_CANDIDATE_SQL, ClaimedJob, FixedUtcInterval, JobCancellation, JobError, JobFuture,
    JobHandler, JobStatus, JobStore as _, JobWakeup, MAX_REQUEUE_BATCH_SIZE, NewJob,
    PostgresJobStore, RecurringJob, StoreError, WorkerConfig, WorkerRunner, spawn_enqueue_listener,
};
use baukit_runtime::ShutdownToken;
use jiff::Timestamp;
use jiff_sqlx::ToSqlx;
use serde_json::json;
use sqlx::PgPool;
use tokio::{sync::Notify, time::Instant};
use uuid::Uuid;

const DELIVERY: &str = "suite.delivery";
const HANGING_PEER: &str = "peer-hanging";
const HEALTHY_PEER: &str = "peer-healthy";
const JOBS_PER_PEER: usize = 10;
const WAIT_LIMIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);
const LISTEN_LATENCY_BOUND: Duration = Duration::from_millis(500);

struct PeerHandler {
    completed: AtomicUsize,
    hanging: AtomicUsize,
    peak_hanging: AtomicUsize,
    release: Notify,
}

impl PeerHandler {
    fn new() -> Self {
        Self {
            completed: AtomicUsize::new(0),
            hanging: AtomicUsize::new(0),
            peak_hanging: AtomicUsize::new(0),
            release: Notify::new(),
        }
    }
}

impl JobHandler for PeerHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[DELIVERY]
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            if job.concurrency_key.as_deref() == Some(HANGING_PEER) {
                let now = self.hanging.fetch_add(1, Ordering::SeqCst) + 1;
                self.peak_hanging.fetch_max(now, Ordering::SeqCst);
                self.release.notified().await;
                self.hanging.fetch_sub(1, Ordering::SeqCst);
                return Ok(());
            }
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

fn worker(id: &str) -> WorkerConfig {
    WorkerConfig {
        worker_id: id.to_owned(),
        queue: "reliability",
        concurrency: 5,
        poll_interval: Duration::from_millis(20),
        idle_poll_max: Duration::from_millis(200),
        ..WorkerConfig::default()
    }
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_hanging_key_holds_at_most_max_per_key_slots() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    for peer in [HANGING_PEER, HEALTHY_PEER] {
        for _ in 0..JOBS_PER_PEER {
            store
                .enqueue(NewJob::new(DELIVERY, json!({}), 3).concurrency_key(peer))
                .await?;
        }
    }
    let handler = Arc::new(PeerHandler::new());
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let config = WorkerConfig {
        max_per_key: NonZeroU32::new(2),
        ..worker("fairness")
    };
    let runner = WorkerRunner::new(Arc::new(store.clone()), handler.clone(), config)?;
    let task = tokio::spawn(runner.run(shutdown.clone()));
    wait_until(|| handler.completed.load(Ordering::SeqCst) == JOBS_PER_PEER).await;
    assert_eq!(handler.peak_hanging.load(Ordering::SeqCst), 2);
    let running: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM job_outbox WHERE concurrency_key = $1 AND status = 'running'",
    )
    .bind(HANGING_PEER)
    .fetch_one(&pool)
    .await?;
    assert_eq!(running, 2);
    shutdown.trigger();
    handler.release.notify_waiters();
    task.await??;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_claims_never_exceed_the_key_limit() -> Result<(), Box<dyn Error>> {
    let (_fixture, _pool, store) = fixture().await?;
    for _ in 0..JOBS_PER_PEER {
        store
            .enqueue(NewJob::new(DELIVERY, json!({}), 3).concurrency_key(HANGING_PEER))
            .await?;
    }
    let limit = NonZeroU32::new(1);
    let now = Timestamp::now();
    let claims = (0..JOBS_PER_PEER).map(|index| {
        let store = store.clone();
        tokio::spawn(async move {
            store
                .claim_with_key_limit(
                    &format!("racer-{index}"),
                    &[DELIVERY],
                    now,
                    Duration::from_secs(60),
                    limit,
                )
                .await
        })
    });
    let mut claimed = 0;
    for claim in claims.collect::<Vec<_>>() {
        if claim.await??.is_some() {
            claimed += 1;
        }
    }
    assert_eq!(claimed, 1);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_claim_counts_running_keys_through_the_partial_index() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    for index in 0..200 {
        store
            .enqueue(NewJob::new(DELIVERY, json!({}), 3).concurrency_key(format!("peer-{index}")))
            .await?;
    }
    sqlx::query("ANALYZE job_outbox").execute(&pool).await?;
    let mut connection = pool.acquire().await?;
    sqlx::query("SET enable_seqscan = off")
        .execute(&mut *connection)
        .await?;
    let empty: &[Uuid] = &[];
    let job_types = vec![DELIVERY.to_owned()];
    let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "EXPLAIN {CLAIM_CANDIDATE_SQL}"
    )))
    .bind(Timestamp::now().to_sqlx())
    .bind(Some(1_i32))
    .bind(&job_types)
    .bind(empty)
    .fetch_all(&mut *connection)
    .await?;
    let plan = plan.join("\n");
    println!("claim candidate plan:\n{plan}");
    assert!(plan.contains("job_outbox_claim_idx"), "{plan}");
    assert!(plan.contains("job_outbox_running_key_idx"), "{plan}");
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn requeue_failed_resets_only_failed_rows_of_the_type() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    let failed = fail_job(&store, NewJob::new(DELIVERY, json!({}), 1)).await?;
    let other_type = fail_job(&store, NewJob::new("other.job", json!({}), 1)).await?;
    let pending = store
        .enqueue(NewJob::new(DELIVERY, json!({}), 1))
        .await?
        .job
        .id;
    let requeued = store
        .requeue_failed(DELIVERY, &[failed, other_type, pending, Uuid::now_v7()])
        .await?;
    assert_eq!(requeued, [failed]);
    let row: (String, i32, i32, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, attempts, deferrals, failure_reason, last_error FROM job_outbox WHERE id = $1",
    )
    .bind(failed)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row, ("pending".to_owned(), 0, 0, None, None));
    assert_eq!(status(&pool, other_type).await?, "failed");
    assert!(
        store
            .claim(
                "requeue-worker",
                &[DELIVERY],
                Timestamp::now(),
                Duration::from_secs(60)
            )
            .await?
            .is_some()
    );

    assert!(matches!(
        store.requeue_failed(DELIVERY, &[]).await,
        Err(StoreError::InvalidInput(_))
    ));
    let too_many = vec![failed; MAX_REQUEUE_BATCH_SIZE + 1];
    assert!(matches!(
        store.requeue_failed(DELIVERY, &too_many).await,
        Err(StoreError::InvalidInput(_))
    ));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn requeue_keeps_a_failed_row_whose_dedupe_key_is_active() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    let first = store
        .enqueue_deduplicated(NewJob::new(DELIVERY, json!({}), 1), "key")
        .await?
        .job
        .id;
    let failed = fail_claimed(&store, first).await?;
    let active = store
        .enqueue_deduplicated(NewJob::new(DELIVERY, json!({}), 1), "key")
        .await?;
    assert!(active.created);
    assert!(store.requeue_failed(DELIVERY, &[failed]).await?.is_empty());
    assert_eq!(status(&pool, failed).await?, "failed");
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn listen_wakes_an_idle_runner_within_half_a_second() -> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    let handler = Arc::new(PeerHandler::new());
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let wakeup = JobWakeup::new();
    let listener = spawn_enqueue_listener(pool.clone(), wakeup.clone(), shutdown.clone());
    let config = WorkerConfig {
        poll_interval: Duration::from_millis(250),
        idle_poll_max: Duration::from_secs(5),
        ..worker("listener")
    };
    let runner =
        WorkerRunner::new(Arc::new(store.clone()), handler.clone(), config)?.with_wakeup(wakeup);
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::sleep(Duration::from_secs(10)).await;
    let enqueued = Instant::now();
    store.enqueue(NewJob::new(DELIVERY, json!({}), 1)).await?;
    wait_until(|| handler.completed.load(Ordering::SeqCst) == 1).await;
    let latency = enqueued.elapsed();
    println!("enqueue to handled with LISTEN: {latency:?}");
    assert!(latency < LISTEN_LATENCY_BOUND, "{latency:?}");
    shutdown.trigger();
    task.await??;
    listener.await?;
    Ok(())
}

struct AlwaysFails;

impl JobHandler for AlwaysFails {
    fn job_types(&self) -> &'static [&'static str] {
        &["retention.purge"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Err(JobError::retryable("purge failed"))))
    }
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_slot_that_fails_every_attempt_still_schedules_the_next_slot()
-> Result<(), Box<dyn Error>> {
    let (_fixture, pool, store) = fixture().await?;
    let interval = FixedUtcInterval::new(Duration::from_secs(3600))?;
    let recurring = RecurringJob::new("retention.purge", interval, 2);
    let now = Timestamp::now();
    let current = recurring.enqueue_initial(&store, now).await?.job;
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let config = WorkerConfig {
        retry_initial: Duration::from_millis(10),
        retry_max: Duration::from_millis(20),
        ..worker("recurring")
    };
    let runner = WorkerRunner::new(Arc::new(store.clone()), Arc::new(AlwaysFails), config)?
        .with_recurring(recurring)?;
    let task = tokio::spawn(runner.run(shutdown.clone()));
    let next_key = interval
        .next_slot(interval.slot_at(now)?, now)?
        .identifier();
    let next = loop {
        let row: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM job_outbox WHERE job_type = 'retention.purge' AND idempotency_key = $1",
        )
        .bind(&next_key)
        .fetch_optional(&pool)
        .await?;
        if let Some(id) = row {
            break id;
        }
        tokio::time::sleep(POLL).await;
    };
    shutdown.trigger();
    task.await??;
    assert_eq!(status(&pool, current.id).await?, "failed");
    assert_eq!(status(&pool, next).await?, "pending");
    let restarted = RecurringJob::new("retention.purge", interval, 2)
        .enqueue_initial(&store, Timestamp::now())
        .await?;
    assert_eq!(
        restarted.job.id, next,
        "a restart skips past the failed slot"
    );
    assert_eq!(restarted.job.status, JobStatus::Pending);
    Ok(())
}

async fn fail_job(store: &PostgresJobStore, job: NewJob) -> Result<Uuid, Box<dyn Error>> {
    let id = store.enqueue(job).await?.job.id;
    fail_claimed(store, id).await
}

async fn fail_claimed(store: &PostgresJobStore, id: Uuid) -> Result<Uuid, Box<dyn Error>> {
    let now = Timestamp::now();
    let types = ["other.job", DELIVERY];
    let claimed = store
        .claim("failer", &types, now, Duration::from_secs(60))
        .await?
        .ok_or("nothing to claim")?;
    assert_eq!(claimed.id, id);
    store
        .record_failure(id, "failer", false, now, "boom", now)
        .await?;
    Ok(id)
}

async fn status(pool: &PgPool, id: Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
}

async fn wait_until(mut condition: impl FnMut() -> bool) {
    tokio::time::timeout(WAIT_LIMIT, async {
        while !condition() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .expect("condition reached before the wait limit");
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}
