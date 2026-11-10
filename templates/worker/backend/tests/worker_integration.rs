use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

{% if context.auth_enabled %}use baukit_config::Secret;
use baukit_erasure::{
    ErasureService, ErasureState, IdentityDeletionError, IdentityRetention, PostgresErasureStore,
};
{% endif %}use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, ShutdownToken, build_info};
use baukit_telemetry::TelemetryBuilder;

use {{ context.app_crate }}_bin::worker_operations_router;
{% if context.auth_enabled %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_postgres::PostgresProfileErasure;
{% else %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% endif %}use {{ context.app_crate }}_services::ItemService;
use {{ context.app_crate }}_worker::DemoJobHandler;

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for durable worker verification"]
async fn durable_outbox_runs_the_generated_demo_handler() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));
    let item = service
        .create("worker-owner", "worker fixture".to_owned())
        .await?;
    let (job_id, status, payload): (uuid::Uuid, String, serde_json::Value) =
        sqlx::query_as("SELECT id, status, payload FROM job_outbox WHERE idempotency_key = $1")
            .bind(format!("item-created:{}", item.id))
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "pending");
    assert_eq!(payload["item_id"], item.id.to_string());

{% if context.auth_enabled %}    let (fake, erasure, operation) = pending_erasure(&pool).await?;

{% endif %}    let service_info = ServiceInfo::new(
        "{{ context.app_name }}",
        ProcessKind::Worker,
        build_info!(),
        DeploymentEnvironment::Local,
    );
    let telemetry = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .filter("off")
        .init()?;
    let runner = demo_runner(&pool)?;
    runner.ready().await?;
    let (ops, readiness) = worker_operations_router(
        runner.clone(),
        service_info.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        TrafficGate::new(),
    )?;
    baukit_test::assert_ops_router_conformance(&ops, &readiness).await;
    baukit_test::assert_metrics_conformance_with_options(
        telemetry.prometheus_handle().render(),
        baukit_test::MetricsConformanceOptions::new().require_worker_metrics(),
    );

    run_until_succeeded(runner, &pool, job_id).await?;

    let status: String = sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "succeeded");
{% if context.auth_enabled %}    assert_eq!(fake.calls(), ["worker-erasure-subject"]);
    let identity_status: String = sqlx::query_scalar(
        "SELECT status FROM job_outbox WHERE job_type = 'identity.account.delete'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(identity_status, "pending");
    assert_eq!(
        erasure
            .store()
            .status("worker-erasure-subject", operation.operation_id)
            .await?
            .ok_or("erasure operation missing")?
            .status,
        ErasureState::Pending
    );
{% endif %}    telemetry.shutdown()?;
    pool.close().await;
    drop(fixture);
    Ok(())
}
{% if context.auth_enabled %}
#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for durable worker verification"]
async fn retention_job_purges_expired_browser_sessions() -> Result<(), Box<dyn Error>> {
    use {{ context.app_crate }}_worker::retention::RetentionJobHandler;

    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let now = chrono::Utc::now();
    insert_browser_session(&pool, "expired", now - chrono::TimeDelta::days(3)).await?;
    insert_browser_session(&pool, "active", now + chrono::TimeDelta::days(3)).await?;

    let handler = Arc::new(RetentionJobHandler::new(pool.clone())?);
    let job_id = handler.enqueue_initial(now).await?.job.id;
    let runner = WorkerRunner::new(
        Arc::new(PostgresJobStore::new(pool.clone())),
        handler,
        WorkerConfig {
            worker_id: "retention-worker-test".to_owned(),
            concurrency: 1,
            poll_interval: Duration::from_millis(10),
            ..WorkerConfig::default()
        },
    )?;
    run_until_succeeded(runner, &pool, job_id).await?;

    let subjects: Vec<String> =
        sqlx::query_scalar("SELECT subject FROM baukit_auth_browser_sessions ORDER BY subject")
            .fetch_all(&pool)
            .await?;
    assert_eq!(subjects, ["active"]);
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn insert_browser_session(
    pool: &sqlx::PgPool,
    subject: &str,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO baukit_auth_browser_sessions (
             id, secret_hash, family_id, subject, key_version, refresh_token,
             created_at, last_used_at, idle_expires_at, absolute_expires_at
         ) VALUES ($1, $2, $3, $4, 1, $5, now(), now(), $6, $6)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(vec![0_u8; 32])
    .bind(uuid::Uuid::new_v4())
    .bind(subject)
    .bind(vec![0_u8; 16])
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}
{% endif %}
{% if context.auth_enabled %}
async fn pending_erasure(
    pool: &sqlx::PgPool,
) -> Result<
    (
        Arc<baukit_test::FakeIdentityAccountDeleter>,
        ErasureService,
        baukit_erasure::ErasureOutcome,
    ),
    Box<dyn Error>,
> {
    let fake = Arc::new(baukit_test::FakeIdentityAccountDeleter::default());
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    let erasure = ErasureService::new(
        PostgresErasureStore::new(
            pool.clone(),
            Secret::new("worker-test-hash-key-with-at-least-32-bytes".into()),
        )?,
        IdentityRetention::Delete {
            deleter: fake.clone(),
            provider_id: "keycloak".into(),
            inline_timeout: Duration::from_secs(1),
            max_attempts: 3,
        },
    )?;
    let operation = erasure
        .erase(
            "worker-erasure-subject",
            None,
            "worker-erasure-key-001",
            &PostgresProfileErasure,
        )
        .await?;
    assert_eq!(operation.status, ErasureState::Pending);
    Ok((fake, erasure, operation))
}
{% endif %}
fn demo_runner(pool: &sqlx::PgPool) -> Result<WorkerRunner, Box<dyn Error>> {
    Ok(WorkerRunner::new(
        Arc::new(PostgresJobStore::new(pool.clone())),
        Arc::new(DemoJobHandler::new()),
        WorkerConfig {
            worker_id: "generated-worker-test".to_owned(),
            concurrency: 1,
            poll_interval: Duration::from_millis(10),
            lease_duration: Duration::from_secs(5),
            job_timeout: Duration::from_secs(2),
            cancellation_poll_interval: Duration::from_millis(10),
            retry_initial: Duration::from_millis(10),
            retry_max: Duration::from_millis(20),
            ..WorkerConfig::default()
        },
    )?)
}

async fn run_until_succeeded(
    runner: WorkerRunner,
    pool: &sqlx::PgPool,
    job_id: uuid::Uuid,
) -> Result<(), Box<dyn Error>> {
    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let task = tokio::spawn(runner.run(shutdown.clone()));
    let finished = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status: String = sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
                .bind(job_id)
                .fetch_one(pool)
                .await?;
            if status == "succeeded" {
                return Ok::<_, sqlx::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    shutdown.trigger();
    task.await??;
    finished??;
    Ok(())
}
