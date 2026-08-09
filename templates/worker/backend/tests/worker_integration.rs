use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, ShutdownToken, build_info};
use baukit_telemetry::TelemetryBuilder;
use {{ context.app_crate }}_bin::worker_operations_router;
use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_services::ItemService;
use {{ context.app_crate }}_worker::DemoJobHandler;

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for durable worker verification"]
async fn durable_outbox_runs_the_generated_demo_handler() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));
    let item = service.create("worker fixture".to_owned()).await?;
    let (job_id, status, payload): (uuid::Uuid, String, serde_json::Value) =
        sqlx::query_as("SELECT id, status, payload FROM job_outbox WHERE idempotency_key = $1")
            .bind(format!("item-created:{}", item.id))
            .fetch_one(&pool)
            .await?;
    assert_eq!(status, "pending");
    assert_eq!(payload["item_id"], item.id.to_string());

    let service_info = ServiceInfo::new(
        "{{ context.app_name }}",
        ProcessKind::Worker,
        build_info!(),
        DeploymentEnvironment::Local,
    );
    let telemetry = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .filter("off")
        .init()?;
    let runner = WorkerRunner::new(
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
    )?;
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

    let shutdown = ShutdownToken::new(Duration::from_secs(2));
    let task = tokio::spawn(runner.run(shutdown.clone()));
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let status: String = sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
                .bind(job_id)
                .fetch_one(&pool)
                .await
                .expect("job status query succeeds");
            if status == "succeeded" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await?;
    shutdown.trigger();
    task.await??;

    let status: String = sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(&pool)
        .await?;
    assert_eq!(status, "succeeded");
    telemetry.shutdown()?;
    pool.close().await;
    drop(fixture);
    Ok(())
}
