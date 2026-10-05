use std::{
    collections::HashSet,
    error::Error,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError},
    time::Duration,
};

use baukit_jobs::{
    ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler, JobStore as _, NewJob,
    PostgresJobStore, StoreError, WorkerConfig, WorkerRunner,
};
use baukit_runtime::ShutdownToken;
use chrono::{TimeDelta, Utc};
use metrics_exporter_prometheus::PrometheusBuilder;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

const ALPHA: &str = "alpha.job";
const BETA: &str = "beta.job";
const UNHANDLED: &str = "unhandled.job";
const JOBS_PER_TYPE: usize = 20;
const UNHANDLED_AGE_SECONDS: i32 = 3_600;
const WAIT_LIMIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(10);

enum ClaimCancellation {
    AbortRunner,
    ShutdownRunner,
    AbortStore,
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn cancelled_claim_finishes_and_recovers_without_poisoning_the_pool()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, _) = fixture().await?;
    let worker_pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(fixture.connection_url())
        .await?;
    let store = Arc::new(PostgresJobStore::new(worker_pool.clone()));
    for cancellation in [
        ClaimCancellation::AbortRunner,
        ClaimCancellation::ShutdownRunner,
        ClaimCancellation::AbortStore,
    ] {
        let job = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
        let mut lock = pool.begin().await?;
        sqlx::query("LOCK TABLE job_outbox IN SHARE MODE")
            .execute(&mut *lock)
            .await?;
        let shutdown = ShutdownToken::new(Duration::from_secs(5));
        let worker = runner(
            &store,
            Arc::new(BlockingHandler),
            "cancelled-worker",
            "cancelled",
        )?;
        let task = match cancellation {
            ClaimCancellation::AbortStore => {
                let store = Arc::clone(&store);
                tokio::spawn(async move {
                    store
                        .claim(
                            "cancelled-worker",
                            &[ALPHA],
                            Utc::now(),
                            Duration::from_secs(30),
                        )
                        .await
                        .map(|_| ())
                        .map_err(Into::into)
                })
            }
            _ => tokio::spawn(worker.run(shutdown.clone())),
        };
        wait_until(|| async {
            sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE 'UPDATE job_outbox SET status%')")
                .fetch_one(&pool).await.expect("claim lock probe")
        }).await;
        if matches!(cancellation, ClaimCancellation::ShutdownRunner) {
            shutdown.trigger();
            task.await??;
        } else {
            task.abort();
            assert!(task.await.expect_err("caller aborted").is_cancelled());
        }
        lock.commit().await?;
        wait_until(|| async {
            status_and_attempts(&pool, job.id)
                .await
                .expect("committed claim probe")
                .0
                == "running"
        })
        .await;
        assert_eq!(
            sqlx::query_scalar::<_, i32>("SELECT 42")
                .fetch_one(&worker_pool)
                .await?,
            42
        );
        let recovery_time = Utc::now() + TimeDelta::minutes(2);
        let recovered = store
            .claim(
                "recovery-worker",
                &[ALPHA],
                recovery_time,
                Duration::from_secs(30),
            )
            .await?
            .expect("abandoned claim recovers after lease expiry");
        assert_eq!(recovered.id, job.id);
        assert_eq!(recovered.attempts, 2);
        assert!(
            store
                .complete(job.id, "recovery-worker", recovery_time)
                .await?
        );
    }
    worker_pool.close().await;
    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_claim_returns_only_requested_job_types() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let unhandled = enqueue_aged(&store, UNHANDLED).await?;
    let alpha = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
    let beta = store.enqueue(NewJob::new(BETA, json!({}), 3)).await?.job;
    let now = Utc::now();
    let lease = Duration::from_secs(30);

    let claimed = store.claim("worker-b", &[BETA], now, lease).await?;
    assert_eq!(claimed.map(|job| job.id), Some(beta.id));
    let claimed = store.claim("worker-a", &[ALPHA, BETA], now, lease).await?;
    assert_eq!(claimed.map(|job| job.id), Some(alpha.id));
    assert!(
        store
            .claim("worker-a", &[ALPHA, BETA], now, lease)
            .await?
            .is_none()
    );
    assert_eq!(
        status_and_attempts(&pool, unhandled).await?,
        ("pending".to_owned(), 0)
    );

    for invalid in [&[][..], &[" "][..]] {
        assert!(matches!(
            store.claim("worker-a", invalid, now, lease).await,
            Err(StoreError::InvalidInput(_))
        ));
    }

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_disjoint_runners_never_claim_each_others_jobs() -> Result<(), Box<dyn Error>> {
    let recorder = PrometheusBuilder::new().build_recorder();
    let metrics = recorder.handle();
    let _guard = baukit_telemetry::metrics::set_default_local_recorder(&recorder);
    let (fixture, pool, store) = fixture().await?;
    let unhandled = enqueue_aged(&store, UNHANDLED).await?;
    let mut expected_alpha = HashSet::new();
    let mut expected_beta = HashSet::new();
    for sequence in 0..JOBS_PER_TYPE {
        let payload = json!({ "sequence": sequence });
        expected_alpha.insert(
            store
                .enqueue(NewJob::new(ALPHA, payload.clone(), 3))
                .await?
                .job
                .id,
        );
        expected_beta.insert(store.enqueue(NewJob::new(BETA, payload, 3)).await?.job.id);
    }

    let alpha = Arc::new(RecordingHandler::new(&[ALPHA]));
    let beta = Arc::new(RecordingHandler::new(&[BETA]));
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let store = Arc::new(store);
    let runners = [
        tokio::spawn(runner(&store, alpha.clone(), "alpha-worker", "alpha")?.run(shutdown.clone())),
        tokio::spawn(runner(&store, beta.clone(), "beta-worker", "beta")?.run(shutdown.clone())),
    ];

    wait_until(|| async {
        succeeded_count(&pool).await.expect("status query succeeds") == 2 * JOBS_PER_TYPE
    })
    .await;
    shutdown.trigger();
    for task in runners {
        task.await??;
    }

    assert_eq!(alpha.seen(), expected_alpha);
    assert_eq!(beta.seen(), expected_beta);
    assert_eq!(
        status_and_attempts(&pool, unhandled).await?,
        ("pending".to_owned(), 0)
    );
    let rendered = metrics.render();
    assert!(
        rendered.contains(r#"worker_job_runs_total{job_kind="unknown",outcome="success"} 0"#),
        "no runner handled an undeclared type:\n{rendered}"
    );
    for queue in ["alpha", "beta"] {
        let age = gauge_value(&rendered, "worker_queue_oldest_age_seconds", queue)
            .expect("queue age gauge is exported");
        assert!(
            age >= f64::from(UNHANDLED_AGE_SECONDS - 60),
            "the unhandled pending job keeps queue {queue} aged, got {age}"
        );
    }

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_filtered_claim_reclaims_expired_leases_only_for_its_types()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
    let now = Utc::now();
    let expired = now + TimeDelta::seconds(2);

    store
        .claim("worker-a", &[ALPHA], now, Duration::from_secs(1))
        .await?
        .expect("first claim");
    assert!(
        store
            .claim("worker-b", &[BETA], expired, Duration::from_secs(10))
            .await?
            .is_none(),
        "an expired lease of another type is not reclaimed"
    );
    assert_eq!(locked_by(&pool, job.id).await?.as_deref(), Some("worker-a"));
    let reclaimed = store
        .claim("worker-c", &[ALPHA], expired, Duration::from_secs(10))
        .await?
        .expect("expired lease is reclaimed by a matching worker");
    assert_eq!(reclaimed.id, job.id);
    assert_eq!(reclaimed.attempts, 2);
    assert_eq!(reclaimed.locked_by.as_deref(), Some("worker-c"));

    let final_attempt = store.enqueue(NewJob::new(ALPHA, json!({}), 1)).await?.job;
    let final_now = Utc::now();
    store
        .claim("worker-c", &[ALPHA], final_now, Duration::from_secs(1))
        .await?
        .expect("final attempt claimed");
    assert!(
        store
            .claim(
                "worker-b",
                &[BETA],
                final_now + TimeDelta::seconds(2),
                Duration::from_secs(10),
            )
            .await?
            .is_none()
    );
    assert_eq!(
        status_and_attempts(&pool, final_attempt.id).await?,
        ("failed".to_owned(), 1),
        "lease recovery finishes an exhausted attempt without running a handler"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_filtered_claim_honors_cancellation() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let pending = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
    let now = Utc::now();
    assert!(store.request_cancellation(pending.id, now).await?);
    assert!(
        store
            .claim("worker-a", &[ALPHA], now, Duration::from_secs(30))
            .await?
            .is_none(),
        "a cancelled pending job is not claimable"
    );
    assert_eq!(
        status_and_attempts(&pool, pending.id).await?,
        ("cancelled".to_owned(), 0)
    );

    let expired = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
    let now = Utc::now();
    store
        .claim("worker-a", &[ALPHA], now, Duration::from_secs(1))
        .await?
        .expect("job claimed");
    assert!(store.request_cancellation(expired.id, now).await?);
    assert!(
        store
            .claim(
                "worker-b",
                &[ALPHA],
                now + TimeDelta::seconds(2),
                Duration::from_secs(10),
            )
            .await?
            .is_none(),
        "an expired lease with a cancellation request is cancelled, not reclaimed"
    );
    assert_eq!(
        status_and_attempts(&pool, expired.id).await?,
        ("cancelled".to_owned(), 1)
    );

    let unhandled = enqueue_aged(&store, UNHANDLED).await?;
    let running = store.enqueue(NewJob::new(ALPHA, json!({}), 3)).await?.job;
    let handler = Arc::new(BlockingHandler);
    let shutdown = ShutdownToken::new(Duration::from_secs(5));
    let store = Arc::new(store);
    let task =
        tokio::spawn(runner(&store, handler, "alpha-worker", "alpha")?.run(shutdown.clone()));
    wait_until(|| async {
        status_and_attempts(&pool, running.id)
            .await
            .expect("status query succeeds")
            .0
            == "running"
    })
    .await;
    assert!(store.request_cancellation(running.id, Utc::now()).await?);
    wait_until(|| async {
        status_and_attempts(&pool, running.id)
            .await
            .expect("status query succeeds")
            .0
            == "cancelled"
    })
    .await;
    shutdown.trigger();
    task.await??;
    assert_eq!(
        status_and_attempts(&pool, unhandled).await?,
        ("pending".to_owned(), 0)
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_claim_index_skips_a_backlog_of_unhandled_types() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, _store) = fixture().await?;
    sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, max_attempts, run_after, created_at, updated_at) SELECT gen_random_uuid(), $1, '{}', 3, now() - interval '1 day' + g * interval '1 second', now() - interval '1 day', now() - interval '1 day' FROM generate_series(1, 50000) AS g",
    )
    .bind(UNHANDLED)
    .execute(&pool)
    .await?;
    sqlx::query(
        "INSERT INTO job_outbox (id, job_type, payload, max_attempts, run_after, created_at, updated_at) SELECT gen_random_uuid(), $1, '{}', 3, now() - interval '1 hour', now() - interval '1 hour', now() - interval '1 hour' FROM generate_series(1, 100)",
    )
    .bind(ALPHA)
    .execute(&pool)
    .await?;
    sqlx::query("ANALYZE job_outbox").execute(&pool).await?;

    let plan: Vec<String> = sqlx::query_scalar(
        "EXPLAIN SELECT id FROM job_outbox WHERE attempts < max_attempts AND cancel_requested_at IS NULL AND job_type = ANY($1) AND ((status = 'pending' AND run_after <= $2) OR (status = 'running' AND locked_until <= $2)) ORDER BY run_after, created_at, id FOR UPDATE SKIP LOCKED LIMIT 1",
    )
    .bind(&[ALPHA][..])
    .bind(Utc::now())
    .fetch_all(&pool)
    .await?;
    let plan = plan.join("\n");
    assert!(plan.contains("job_outbox_claim_idx"), "{plan}");
    assert!(!plan.contains("Seq Scan"), "{plan}");

    pool.close().await;
    drop(fixture);
    Ok(())
}

struct RecordingHandler {
    job_types: &'static [&'static str],
    seen: Mutex<HashSet<Uuid>>,
}

impl RecordingHandler {
    fn new(job_types: &'static [&'static str]) -> Self {
        Self {
            job_types,
            seen: Mutex::new(HashSet::new()),
        }
    }

    fn seen(&self) -> HashSet<Uuid> {
        self.seen
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl JobHandler for RecordingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        self.job_types
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            if !self.job_types.contains(&job.job_type.as_str()) {
                return Err(JobError::permanent("claimed an undeclared job type"));
            }
            let first = self
                .seen
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(job.id);
            if first {
                Ok(())
            } else {
                Err(JobError::permanent("claimed a job twice"))
            }
        })
    }
}

struct BlockingHandler;

impl JobHandler for BlockingHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[ALPHA]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            cancellation.cancelled().await;
            Err(JobError::retryable("cancelled"))
        })
    }
}

fn runner(
    store: &Arc<PostgresJobStore>,
    handler: Arc<dyn JobHandler>,
    worker_id: &str,
    queue: &'static str,
) -> Result<WorkerRunner, baukit_jobs::RunnerError> {
    WorkerRunner::new(
        Arc::clone(store) as Arc<dyn baukit_jobs::JobStore>,
        handler,
        WorkerConfig {
            worker_id: worker_id.to_owned(),
            queue,
            concurrency: 2,
            poll_interval: POLL,
            lease_duration: Duration::from_secs(30),
            job_timeout: Duration::from_secs(10),
            cancellation_poll_interval: POLL,
            retry_initial: POLL,
            retry_max: Duration::from_secs(1),
        },
    )
}

async fn wait_until<F, Fut>(mut condition: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    tokio::time::timeout(WAIT_LIMIT, async {
        while !condition().await {
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

async fn enqueue_aged(store: &PostgresJobStore, job_type: &str) -> Result<Uuid, StoreError> {
    let created_at = Utc::now() - TimeDelta::seconds(i64::from(UNHANDLED_AGE_SECONDS));
    let mut job = NewJob::new(job_type, json!({}), 3);
    job.created_at = created_at;
    job.run_after = created_at;
    Ok(store.enqueue(job).await?.job.id)
}

async fn status_and_attempts(pool: &PgPool, job_id: Uuid) -> Result<(String, i32), sqlx::Error> {
    sqlx::query_as("SELECT status, attempts FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn locked_by(pool: &PgPool, job_id: Uuid) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar("SELECT locked_by FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn succeeded_count(pool: &PgPool) -> Result<usize, Box<dyn Error>> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_outbox WHERE status = 'succeeded'")
            .fetch_one(pool)
            .await?;
    Ok(usize::try_from(count)?)
}

fn gauge_value(rendered: &str, name: &str, queue: &str) -> Option<f64> {
    let prefix = format!(r#"{name}{{queue="{queue}"}} "#);
    rendered
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .and_then(|value| value.trim().parse().ok())
}
