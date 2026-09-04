use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

{% if context.auth_oidc %}use axum::{extract::Request, http::Method, middleware};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};
{% endif %}use baukit_config::{BaukitConfig, ConfigLoader, Environment};
use baukit_ops::{PoolMetricsSampler, TrafficGate, spawn_pool_metrics_sampler};
{% if context.auth_oidc %}use baukit_ratelimit::{
    AuthenticatedRouteGroupOptions, Quota, RateLimitOptions, RedisRateLimitStore,
};
{% endif %}use baukit_runtime::{ProcessKind, ServiceInfo, ShutdownToken, build_info, serve_listener_pair};
use baukit_telemetry::{TelemetryBuilder, tracing};

use {{ context.app_crate }}_api::{ApiState, router};
{% if context.auth_oidc %}use {{ context.app_crate }}_bin::InMemoryItemRepository;
use {{ context.app_crate }}_bin::InMemoryUserRepository;
use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::operations_router;
{% else %}use {{ context.app_crate }}_bin::InMemoryItemRepository;
use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::operations_router;
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
{% if context.auth_oidc %}const ITEM_WRITE_GROUP: &str = "item_writes";
const ITEM_WRITE_REQUESTS_PER_MINUTE: u64 = 30;
{% endif %}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var("{{ context.app_env }}_ENVIRONMENT")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config: BaukitConfig<ProductConfig> = ConfigLoader::new(PRODUCT, environment)?.load()?;
    run(config).await
}

async fn run(config: BaukitConfig<ProductConfig>) -> Result<(), Box<dyn Error>> {
    let service_info =
        ServiceInfo::new(PRODUCT, ProcessKind::Api, build_info!(), config.environment);
    let mut telemetry_builder = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .sampling_ratio(config.telemetry.trace_sampling_ratio)
        .log_format(config.telemetry.log_format);
    if let Some(endpoint) = &config.telemetry.otlp_endpoint {
        telemetry_builder = telemetry_builder.otlp_endpoint(endpoint);
    }
    let telemetry = Arc::new(telemetry_builder.init()?);

{% if context.auth_oidc %}    let (item_repository, user_repository, pool_metrics): (
        Arc<dyn ItemRepository>,
        Arc<dyn UserRepository>,
        Option<PoolMetricsSampler>,
    ) = if let Some(database) = &config.database {
        let pool = PgPoolOptions::new()
            .max_connections(database.max_connections)
            .min_connections(database.min_connections)
            .acquire_timeout(database.acquire_timeout)
            .connect(database.url.expose())
            .await?;
        let pool_metrics = spawn_pool_metrics_sampler(pool.clone(), Duration::from_secs(15))?;
        (
            Arc::new(PostgresItemRepository::new(pool.clone())),
            Arc::new(PostgresUserRepository::new(pool)),
            Some(pool_metrics),
        )
    } else {
        tracing::warn!(message = "database is not configured; using the in-memory item adapter");
        (
            Arc::new(InMemoryItemRepository::new()),
            Arc::new(InMemoryUserRepository::new()),
            None,
        )
    };
    let item_service = ItemService::new(item_repository);
    let user_service = UserService::new(user_repository);
    let oidc = OidcConfig::new(&config.product.auth.issuer, &config.product.auth.audience)?;
    let auth = AuthState::new(OidcVerifier::discover(oidc).await?);
{% else %}    let (repository, pool_metrics): (Arc<dyn ItemRepository>, Option<PoolMetricsSampler>) =
        if let Some(database) = &config.database {
            let pool = PgPoolOptions::new()
                .max_connections(database.max_connections)
                .min_connections(database.min_connections)
                .acquire_timeout(database.acquire_timeout)
                .connect(database.url.expose())
                .await?;
            let pool_metrics = spawn_pool_metrics_sampler(pool.clone(), Duration::from_secs(15))?;
            (
                Arc::new(PostgresItemRepository::new(pool)),
                Some(pool_metrics),
            )
        } else {
            tracing::warn!(
                message = "database is not configured; using the in-memory item adapter"
            );
            (Arc::new(InMemoryItemRepository::new()), None)
        };
    let item_service = ItemService::new(repository);
{% endif %}
    let api = router(
        ApiState {
            items: item_service.clone(),
{% if context.auth_oidc %}            users: user_service,
            auth: auth.clone(),
{% endif %}        },
        &config.http,
    )?;
{% if context.auth_oidc %}    let rate_limit_options = RateLimitOptions::from_config(&config.rate_limit)?;
    let rate_limit_store = RedisRateLimitStore::connect_if_enabled(&rate_limit_options).await?;
    let api = if let Some(store) = rate_limit_store {
        let item_write_options = AuthenticatedRouteGroupOptions::new(
            ITEM_WRITE_GROUP,
            Quota::new(ITEM_WRITE_REQUESTS_PER_MINUTE, Duration::from_secs(60), 0)?,
            &rate_limit_options,
        )?;
        let api = baukit_ratelimit::authenticated_route_group(
            api,
            store.clone(),
            item_write_options,
            item_write_subject,
            is_item_write,
        );
        baukit_ratelimit::layers(api, store, rate_limit_options)
    } else {
        api
    };
    // Axum runs the last added layer first. Authentication establishes Principal
    // before the inner rate limiter chooses an identity or IP bucket.
    let api = api.layer(middleware::from_fn_with_state(
        auth,
        baukit_auth::establish_principal,
    ));
{% endif %}    let shutdown = ShutdownToken::new(config.shutdown.drain_timeout);
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
    if let Some(pool_metrics) = pool_metrics {
        pool_metrics.shutdown().await;
    }
    let telemetry_for_shutdown = Arc::clone(&telemetry);
    shutdown
        .run_during_drain(async move {
            tokio::task::spawn_blocking(move || telemetry_for_shutdown.shutdown()).await
        })
        .await???;
    result?;
    Ok(())
}{% if context.auth_oidc %}

fn item_write_subject(principal: &Principal) -> String {
    principal.subject().to_owned()
}

fn is_item_write(request: &Request) -> bool {
    matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}{% endif %}
