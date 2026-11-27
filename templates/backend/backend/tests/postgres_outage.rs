use std::{
    error::Error,
    path::PathBuf,
    sync::{Arc, LazyLock},
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
};
use baukit_config::{DatabaseConfig, Secret};
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, build_info};
use baukit_telemetry::{PrometheusHandle, TelemetryBuilder};
use baukit_test::{PostgresMigrationOwner, PostgresTestContainer, PostgresTestOptions};
use sqlx::PgPool;
use tower::ServiceExt as _;

use {{ context.app_crate }}_bin::operations_router;
use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_services::ItemService;

const OWNER: &str = "outage-owner";
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(2);
/// The API answers within the pool queue timeout plus this slack while PostgreSQL is down.
const ANSWER_SLACK: Duration = Duration::from_secs(3);
const RECOVERY_BUDGET: Duration = Duration::from_secs(30);
const LATE_START_DELAY: Duration = Duration::from_secs(5);
const STARTUP_BUDGET: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(250);

async fn start_postgres() -> Result<PostgresTestContainer, Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    Ok(PostgresTestOptions::new()
        .with_stable_host_port()
        .with_migrations(migrations)
        .with_migration_owner(PostgresMigrationOwner::default(){% if context.postgres_owner_create_role %}.with_create_role(){% endif %})
        .start()
        .await?)
}

fn database_config(postgres: &PostgresTestContainer) -> Result<DatabaseConfig, Box<dyn Error>> {
    let url = postgres.owner_connection_url().ok_or("missing owner URL")?;
    Ok(DatabaseConfig {
        url: Secret::new(url.to_owned()),
        min_connections: 0,
        acquire_timeout: ACQUIRE_TIMEOUT,
        ..DatabaseConfig::default()
    })
}

fn service_info() -> ServiceInfo {
    ServiceInfo::new(
        "{{ context.app_name }}",
        ProcessKind::Api,
        build_info!(),
        DeploymentEnvironment::Local,
    )
}

/// Telemetry installs process-global recorders, so every test in this binary shares one handle.
static PROMETHEUS: LazyLock<Result<PrometheusHandle, String>> = LazyLock::new(|| {
    TelemetryBuilder::new(service_info().telemetry_identity().clone())
        .filter("off")
        .init()
        .map(|telemetry| telemetry.prometheus_handle().clone())
        .map_err(|error| error.to_string())
});

fn prometheus_handle() -> Result<PrometheusHandle, Box<dyn Error>> {
    Ok(PROMETHEUS.as_ref().map_err(Clone::clone)?.clone())
}

fn ops_router(pool: &PgPool) -> Result<Router, Box<dyn Error>> {
    let service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));
    let (router, _readiness) = operations_router(
        service,
        service_info().telemetry_identity().clone(),
        prometheus_handle()?,
        TrafficGate::new(),
    )?;
    Ok(router)
}

async fn status(router: &Router, path: &str) -> Result<StatusCode, Box<dyn Error>> {
    let response = router
        .clone()
        .oneshot(Request::get(path).body(Body::empty())?)
        .await?;
    Ok(response.status())
}

async fn wait_for_ready(router: &Router) -> Result<(), Box<dyn Error>> {
    tokio::time::timeout(RECOVERY_BUDGET, async {
        while status(router, "/readyz").await? != StatusCode::OK {
            tokio::time::sleep(POLL).await;
        }
        Ok::<_, Box<dyn Error>>(())
    })
    .await
    .map_err(|_| "readiness did not recover")?
}

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; stops PostgreSQL under a live pool"]
async fn a_postgres_outage_fails_readiness_and_recovers_without_a_restart()
-> Result<(), Box<dyn Error>> {
    let postgres = start_postgres().await?;
    let database = database_config(&postgres)?;
    let pool = baukit_ops::connect_with_backoff(&database, STARTUP_BUDGET).await?;
    let ops = ops_router(&pool)?;
    let service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));
    assert_eq!(status(&ops, "/readyz").await?, StatusCode::OK);

    postgres.fault()?.stop().await?;
    assert_eq!(status(&ops, "/healthz").await?, StatusCode::OK);
    assert_eq!(
        status(&ops, "/readyz").await?,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let started = tokio::time::Instant::now();
    let outage = service.list(OWNER).await;
    assert!(outage.is_err(), "a stopped database must fail the request");
    assert!(
        started.elapsed() <= ACQUIRE_TIMEOUT + ANSWER_SLACK,
        "the request must fail within the pool queue timeout"
    );
{% if not context.auth_enabled %}    let api = {{ context.app_crate }}_api::routes({{ context.app_crate }}_api::ApiState {
        items: service.clone(),
    });
    let response = api
        .oneshot(Request::get("/items").body(Body::empty())?)
        .await?;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response
            .headers()
            .contains_key(axum::http::header::RETRY_AFTER),
        "a 503 must name Retry-After"
    );
{% endif %}
    postgres.fault()?.start().await?;
    wait_for_ready(&ops).await?;
    assert!(service.list(OWNER).await?.is_empty());
    Ok(())
}

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; starts PostgreSQL after the API connects"]
async fn the_api_waits_for_a_late_postgres_and_turns_ready() -> Result<(), Box<dyn Error>> {
    let postgres = start_postgres().await?;
    let database = database_config(&postgres)?;
    postgres.fault()?.stop().await?;

    let connect =
        tokio::spawn(
            async move { baukit_ops::connect_with_backoff(&database, STARTUP_BUDGET).await },
        );
    tokio::time::sleep(LATE_START_DELAY).await;
    assert!(
        !connect.is_finished(),
        "the API must keep waiting for PostgreSQL"
    );
    postgres.fault()?.start().await?;

    let pool = connect.await??;
    wait_for_ready(&ops_router(&pool)?).await
}
