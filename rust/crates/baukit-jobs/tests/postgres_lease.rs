#![cfg(test)]
use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{
    ClaimedJob, FailureDisposition, JobCancellation, JobError, JobFuture, JobHandler,
    JobStore as _, NewJob, PostgresJobStore, QueueDepth, WorkerConfig, WorkerRunner,
};
use baukit_runtime::ShutdownToken;
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::{PgPool, postgres::PgPoolOptions};

const KIND: &str = "lease.job";
const OTHER_KIND: &str = "other.job";
const LEASE: Duration = Duration::from_secs(30);
const WAIT_LIMIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(20);

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_heartbeat_extends_only_an_owned_unexpired_lease() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 3)).await?.job;
    let now = Timestamp::now();
    store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("claimed");

    let later = now + SignedDuration::from_secs(20);
    assert!(store.heartbeat(job.id, "worker-a", LEASE, later).await?);
    let renewed = locked_until(&pool, job.id)
        .await?
        .expect("running job has a lease");
    let drift = renewed
        .duration_since(later + SignedDuration::from_secs(30))
        .abs();
    assert!(
        drift < SignedDuration::from_millis(1),
        "lease renewed to now + lease"
    );
    assert!(!store.heartbeat(job.id, "worker-b", LEASE, later).await?);
    let expired = later + SignedDuration::from_secs(31);
    assert!(!store.heartbeat(job.id, "worker-a", LEASE, expired).await?);
    assert!(
        store
            .claim(
                "worker-b",
                &[KIND],
                now + SignedDuration::from_secs(40),
                LEASE
            )
            .await?
            .is_none(),
        "the renewed lease keeps the job from being reclaimed"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_release_without_attempt_gives_it_back_and_counts_a_deferral()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 1)).await?.job;
    let now = Timestamp::now();
    store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("final attempt claimed");
    let run_after = now + SignedDuration::from_secs(5);

    assert_eq!(
        store
            .release(job.id, "worker-b", run_after, false, now)
            .await?,
        None,
        "another worker cannot release the job"
    );
    assert_eq!(
        store
            .release(job.id, "worker-a", run_after, false, now)
            .await?,
        Some(FailureDisposition::Retry)
    );
    let row = counters(&pool, job.id).await?;
    assert_eq!(row, ("pending".to_owned(), 0, 1));
    assert!(
        store
            .claim("worker-b", &[KIND], now, LEASE)
            .await?
            .is_none()
    );
    let reclaimed = store
        .claim("worker-b", &[KIND], run_after, LEASE)
        .await?
        .expect("released job is claimable at run_after");
    assert_eq!((reclaimed.attempts, reclaimed.deferrals), (1, 1));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_release_with_attempt_retries_and_fails_on_the_final_attempt()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 2)).await?.job;
    let now = Timestamp::now();
    store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("first");
    assert_eq!(
        store.release(job.id, "worker-a", now, true, now).await?,
        Some(FailureDisposition::Retry)
    );
    assert_eq!(counters(&pool, job.id).await?, ("pending".to_owned(), 1, 0));

    store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("second");
    assert_eq!(
        store.release(job.id, "worker-a", now, true, now).await?,
        Some(FailureDisposition::Failed),
        "a deferral past the cap on the final attempt fails the job"
    );
    assert_eq!(counters(&pool, job.id).await?, ("failed".to_owned(), 2, 0));
    let reason: Option<String> =
        sqlx::query_scalar("SELECT failure_reason FROM job_outbox WHERE id = $1")
            .bind(job.id)
            .fetch_one(&pool)
            .await?;
    assert_eq!(reason.as_deref(), Some("attempts_exhausted"));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_release_leaves_a_cancellation_request_to_the_runner() -> Result<(), Box<dyn Error>>
{
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 3)).await?.job;
    let now = Timestamp::now();
    store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("claimed");
    assert!(store.request_cancellation(job.id, now).await?);
    assert_eq!(
        store.release(job.id, "worker-a", now, false, now).await?,
        None
    );
    assert_eq!(counters(&pool, job.id).await?, ("running".to_owned(), 1, 0));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_runner_deferrals_stop_being_free_at_the_cap() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 3)).await?.job;
    let shutdown = ShutdownToken::new(WAIT_LIMIT);
    let runner = WorkerRunner::new(
        Arc::new(store.clone()),
        Arc::new(DeferringHandler),
        WorkerConfig {
            max_deferrals: 2,
            ..config("deferring-worker")
        },
    )?;
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(WAIT_LIMIT, async {
        while counters(&pool, job.id).await?.0 != "failed" {
            tokio::time::sleep(POLL).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;
    shutdown.trigger();
    task.await??;

    assert_eq!(
        counters(&pool, job.id).await?,
        ("failed".to_owned(), 3, 2),
        "two free deferrals, then each deferral consumes one of three attempts"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_queue_age_ignores_future_rows_and_other_types() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Timestamp::now();
    let mut future_slot = NewJob::new(KIND, json!({}), 3);
    future_slot.created_at = now - SignedDuration::from_hours(23);
    future_slot.run_after = now + SignedDuration::from_hours(1);
    store.enqueue(future_slot).await?;
    let mut other = NewJob::new(OTHER_KIND, json!({}), 3);
    other.created_at = now - SignedDuration::from_hours(2);
    other.run_after = now - SignedDuration::from_hours(2);
    store.enqueue(other).await?;

    assert_eq!(
        store.oldest_pending_age(&[KIND], now).await?,
        Duration::ZERO
    );

    let mut ready = NewJob::new(KIND, json!({}), 3);
    ready.created_at = now - SignedDuration::from_hours(5);
    ready.run_after = now - SignedDuration::from_secs(90);
    store.enqueue(ready).await?;
    assert_eq!(
        store.oldest_pending_age(&[KIND], now).await?,
        Duration::from_secs(90),
        "age counts from run_after, not created_at"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_queue_depth_counts_the_runner_types_by_status() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let now = Timestamp::now();
    for _ in 0..3 {
        store.enqueue(NewJob::new(KIND, json!({}), 1)).await?;
    }
    store.enqueue(NewJob::new(OTHER_KIND, json!({}), 1)).await?;
    let running = store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("first claim");
    let failed = store
        .claim("worker-a", &[KIND], now, LEASE)
        .await?
        .expect("second claim");
    store
        .record_failure(failed.id, "worker-a", false, now, "permanent", now)
        .await?;

    assert_eq!(
        store.queue_depth(&[KIND]).await?,
        QueueDepth {
            pending: 1,
            running: 1,
            failed: 1,
        }
    );
    assert!(store.complete(running.id, "worker-a", now).await?);
    assert_eq!(store.queue_depth(&[KIND]).await?.running, 0);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_shutdown_hands_a_long_final_attempt_to_another_worker()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(KIND, json!({}), 1)).await?.job;
    let first_shutdown = ShutdownToken::new(Duration::from_secs(2));
    let first = WorkerRunner::new(
        Arc::new(store.clone()),
        Arc::new(LongHandler),
        WorkerConfig {
            release_reserve: Duration::from_secs(1),
            ..config("first-worker")
        },
    )?;
    let first_task = tokio::spawn(first.run(first_shutdown.clone()));
    wait_for_status(&pool, job.id, "running").await?;

    first_shutdown.trigger();
    first_task.await??;
    assert_eq!(counters(&pool, job.id).await?, ("pending".to_owned(), 0, 1));

    let second_shutdown = ShutdownToken::new(WAIT_LIMIT);
    let second = WorkerRunner::new(
        Arc::new(store.clone()),
        Arc::new(QuickHandler),
        config("second-worker"),
    )?;
    let claimed_at = std::time::Instant::now();
    let second_task = tokio::spawn(second.run(second_shutdown.clone()));
    wait_for_status(&pool, job.id, "succeeded").await?;
    assert!(
        claimed_at.elapsed() < Duration::from_secs(5),
        "the released job is claimed at once, not after a lease expiry"
    );
    second_shutdown.trigger();
    second_task.await??;
    assert_eq!(
        counters(&pool, job.id).await?.1,
        1,
        "the final attempt was not lost"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_pause_mid_run_keeps_the_runner_and_its_attempt_alive()
-> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_secs(1))
        .connect(fixture.connection_url())
        .await?;
    let store = PostgresJobStore::new(pool.clone());
    let job = store.enqueue(NewJob::new(KIND, json!({}), 1)).await?.job;
    let shutdown = ShutdownToken::new(WAIT_LIMIT);
    let runner = WorkerRunner::new(
        Arc::new(store.clone()),
        Arc::new(SleepingHandler(Duration::from_secs(15))),
        WorkerConfig {
            lease_duration: Duration::from_secs(60),
            ..config("paused-worker")
        },
    )?;
    let health = runner.store_health();
    let task = tokio::spawn(runner.run(shutdown.clone()));
    wait_for_status(&pool, job.id, "running").await?;

    fixture.container().pause().await?;
    tokio::time::timeout(Duration::from_secs(10), async {
        while health.is_healthy() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .expect("store health turns red while Postgres is paused");
    tokio::time::sleep(Duration::from_secs(10)).await;
    assert!(!task.is_finished(), "the runner survives the outage");
    fixture.container().unpause().await?;

    wait_for_status(&pool, job.id, "succeeded").await?;
    tokio::time::timeout(WAIT_LIMIT, async {
        while !health.is_healthy() {
            tokio::time::sleep(POLL).await;
        }
    })
    .await
    .expect("store health recovers");
    assert_eq!(
        counters(&pool, job.id).await?.1,
        1,
        "the attempt was not aborted"
    );
    shutdown.trigger();
    task.await??;

    pool.close().await;
    drop(fixture);
    Ok(())
}

struct DeferringHandler;

impl JobHandler for DeferringHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[KIND]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Err(JobError::deferred(
            "circuit open",
            Duration::ZERO,
        ))))
    }
}

/// Ignores cancellation, like a handler stuck in a slow call.
struct LongHandler;

impl JobHandler for LongHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[KIND]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async {
            tokio::time::sleep(Duration::from_secs(120)).await;
            Ok(())
        })
    }
}

struct SleepingHandler(Duration);

impl JobHandler for SleepingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[KIND]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            tokio::time::sleep(self.0).await;
            Ok(())
        })
    }
}

struct QuickHandler;

impl JobHandler for QuickHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[KIND]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(std::future::ready(Ok(())))
    }
}

fn config(worker_id: &str) -> WorkerConfig {
    WorkerConfig {
        worker_id: worker_id.to_owned(),
        concurrency: 1,
        poll_interval: POLL,
        cancellation_poll_interval: POLL,
        retry_initial: POLL,
        retry_max: POLL,
        ..WorkerConfig::default()
    }
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}

async fn counters(pool: &PgPool, job_id: uuid::Uuid) -> Result<(String, i32, i32), sqlx::Error> {
    sqlx::query_as("SELECT status, attempts, deferrals FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn locked_until(pool: &PgPool, job_id: uuid::Uuid) -> Result<Option<Timestamp>, sqlx::Error> {
    sqlx::query_scalar::<_, Option<jiff_sqlx::Timestamp>>(
        "SELECT locked_until FROM job_outbox WHERE id = $1",
    )
    .bind(job_id)
    .fetch_one(pool)
    .await
    .map(|value| value.map(jiff_sqlx::Timestamp::to_jiff))
}

async fn wait_for_status(
    pool: &PgPool,
    job_id: uuid::Uuid,
    expected: &str,
) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(WAIT_LIMIT, async {
        while counters(pool, job_id).await?.0 != expected {
            tokio::time::sleep(POLL).await;
        }
        Ok::<(), sqlx::Error>(())
    })
    .await??;
    Ok(())
}
