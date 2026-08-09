use std::{env, error::Error, net::SocketAddr, sync::Arc};

{% if context.auth_oidc %}use baukit_auth::{AuthState, OidcConfig, OidcVerifier};
{% endif %}use baukit_config::{BaukitConfig, ConfigLoader, Environment};
use baukit_ops::TrafficGate;
use baukit_runtime::{ProcessKind, ServiceInfo, ShutdownToken, build_info, serve_listener_pair};
use baukit_telemetry::{TelemetryBuilder, tracing};

use {{ context.app_crate }}_api::{ApiState, router};
{% if context.auth_oidc %}use {{ context.app_crate }}_bin::{
    InMemoryItemRepository, InMemoryUserRepository, ProductConfig, operations_router,
};
{% else %}use {{ context.app_crate }}_bin::{InMemoryItemRepository, operations_router};
{% endif %}
{% if context.auth_oidc %}use {{ context.app_crate }}_ports::{ItemRepository, UserRepository};
{% else %}use {{ context.app_crate }}_ports::ItemRepository;
{% endif %}
{% if context.auth_oidc %}use {{ context.app_crate }}_postgres::{PostgresItemRepository, PostgresUserRepository};
{% else %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% endif %}
{% if context.auth_oidc %}use {{ context.app_crate }}_services::{ItemService, UserService};
{% else %}use {{ context.app_crate }}_services::ItemService;
{% endif %}
use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

const PRODUCT: &str = "{{ context.app_name }}";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var("{{ context.app_env }}_ENVIRONMENT")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config: BaukitConfig<{% if context.auth_oidc %}ProductConfig{% else %}(){% endif %}> = ConfigLoader::new(PRODUCT, environment)?.load()?;
    run(config).await
}

async fn run(config: BaukitConfig<{% if context.auth_oidc %}ProductConfig{% else %}(){% endif %}>) -> Result<(), Box<dyn Error>> {
    let service_info =
        ServiceInfo::new(PRODUCT, ProcessKind::Api, build_info!(), config.environment);
    let mut telemetry_builder = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .sampling_ratio(config.telemetry.trace_sampling_ratio)
        .log_format(config.telemetry.log_format);
    if let Some(endpoint) = &config.telemetry.otlp_endpoint {
        telemetry_builder = telemetry_builder.otlp_endpoint(endpoint);
    }
    let telemetry = Arc::new(telemetry_builder.init()?);

{% if context.auth_oidc %}    let (item_repository, user_repository): (Arc<dyn ItemRepository>, Arc<dyn UserRepository>) =
        if let Some(database) = &config.database {
            let pool = PgPoolOptions::new()
                .max_connections(database.max_connections)
                .min_connections(database.min_connections)
                .acquire_timeout(database.acquire_timeout)
                .connect(database.url.expose())
                .await?;
            (
                Arc::new(PostgresItemRepository::new(pool.clone())),
                Arc::new(PostgresUserRepository::new(pool)),
            )
        } else {
            tracing::warn!(
                message = "database is not configured; using the in-memory item adapter"
            );
            (
                Arc::new(InMemoryItemRepository::new()),
                Arc::new(InMemoryUserRepository::new()),
            )
        };
    let item_service = ItemService::new(item_repository);
    let user_service = UserService::new(user_repository);
    let oidc = OidcConfig::new(&config.product.auth.issuer, &config.product.auth.audience)?;
    let auth = AuthState::new(OidcVerifier::discover(oidc).await?);
{% else %}    let repository: Arc<dyn ItemRepository> = if let Some(database) = &config.database {
        let pool = PgPoolOptions::new()
            .max_connections(database.max_connections)
            .min_connections(database.min_connections)
            .acquire_timeout(database.acquire_timeout)
            .connect(database.url.expose())
            .await?;
        Arc::new(PostgresItemRepository::new(pool))
    } else {
        tracing::warn!(message = "database is not configured; using the in-memory item adapter");
        Arc::new(InMemoryItemRepository::new())
    };
    let item_service = ItemService::new(repository);
{% endif %}
    let api = router(
        ApiState {
            items: item_service.clone(),
{% if context.auth_oidc %}            users: user_service,
            auth,
{% endif %}        },
        &config.http,
    )?;

    let shutdown = ShutdownToken::new(config.shutdown.drain_timeout);
    let traffic_gate = TrafficGate::new();
    shutdown.on_drain({
        let traffic_gate = traffic_gate.clone();
        move || traffic_gate.stop_accepting()
    });
    let (operations, _readiness) = operations_router(
        item_service,
        service_info.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        traffic_gate,
    )?;

    let api_listener =
        TcpListener::bind(SocketAddr::new(config.http.bind_address, config.http.port)).await?;
    let operations_listener =
        TcpListener::bind(SocketAddr::new(config.ops.bind_address, config.ops.port)).await?;
    tracing::info!(
        message = "service started",
        api_address = %api_listener.local_addr()?,
        operations_address = %operations_listener.local_addr()?,
    );

    let signal_task = shutdown.spawn_signal_listener();
    let result = serve_listener_pair(
        api_listener,
        api,
        operations_listener,
        operations,
        shutdown.clone(),
    )
    .await;
    shutdown.trigger();
    if !signal_task.is_finished() {
        signal_task.abort();
    }
    let _signal_result = signal_task.await;
    let telemetry_for_shutdown = Arc::clone(&telemetry);
    shutdown
        .run_during_drain(async move {
            tokio::task::spawn_blocking(move || telemetry_for_shutdown.shutdown()).await
        })
        .await???;
    result?;
    Ok(())
}
