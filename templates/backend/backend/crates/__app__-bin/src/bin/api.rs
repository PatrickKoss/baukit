use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

{% if context.auth_oidc %}use axum::{extract::Request, http::Method, middleware};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};
{% endif %}use baukit_config::{BaukitConfig, ConfigLoader, Environment};
{% if context.auth_oidc %}use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
{% endif %}{% if not context.auth_oidc %}use baukit_ops::PoolMetricsSampler;
{% endif %}use baukit_ops::{TrafficGate, spawn_pool_metrics_sampler};
{% if context.auth_oidc %}use baukit_ratelimit::{
    AuthenticatedRouteGroupOptions, Quota, RateLimitOptions, RedisRateLimitStore,
};
{% endif %}use baukit_runtime::{ProcessKind, ServiceInfo, ShutdownToken, build_info, serve_listener_pair};
use baukit_telemetry::{TelemetryBuilder, tracing};

use sqlx::postgres::PgPoolOptions;
use tokio::net::TcpListener;

{% if context.auth_oidc %}use {{ context.app_crate }}_api::ApiState;
use {{ context.app_crate }}_api::ErasureApi;
use {{ context.app_crate }}_api::finalize_api;
use {{ context.app_crate }}_api::routes;
use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::identity_erasure;
use {{ context.app_crate }}_bin::operations_router;
use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_postgres::PostgresProfileErasure;
use {{ context.app_crate }}_postgres::PostgresUserRepository;
use {{ context.app_crate }}_services::ItemService;
use {{ context.app_crate }}_services::UserService;
{% else %}use {{ context.app_crate }}_api::ApiState;
use {{ context.app_crate }}_api::finalize_api;
use {{ context.app_crate }}_api::routes;
use {{ context.app_crate }}_bin::InMemoryItemRepository;
use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::operations_router;
use {{ context.app_crate }}_ports::ItemRepository;
use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_services::ItemService;
{% endif %}
const PRODUCT: &str = "{{ context.app_name }}";
const ENV_PREFIX: &str = "{{ context.app_env }}";
{% if context.auth_oidc %}const ITEM_WRITE_GROUP: &str = "item_writes";
const ITEM_WRITE_REQUESTS_PER_MINUTE: u64 = 30;
{% endif %}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var(format!("{ENV_PREFIX}_ENVIRONMENT"))
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config: BaukitConfig<ProductConfig> = ConfigLoader::new(PRODUCT, environment)?{% if context.mcp %}
        .environment_collection("mcp.allowed_hosts")
        .environment_collection("mcp.allowed_origins")
        {% endif %}.load()?;
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

{% if context.auth_oidc %}    let database = config
        .database
        .as_ref()
        .ok_or("authenticated backend requires database configuration")?;
    let pool = PgPoolOptions::new()
        .max_connections(database.max_connections)
        .min_connections(database.min_connections)
        .acquire_timeout(database.acquire_timeout)
        .connect(database.url.expose())
        .await?;
    let pool_metrics = Some(spawn_pool_metrics_sampler(
        pool.clone(),
        Duration::from_secs(15),
    )?);
    let (erasure_service, erasure_handler) =
        identity_erasure(pool.clone(), &config.product.auth, config.environment)?;
    let erasure_runner = WorkerRunner::new(
        Arc::new(
            PostgresJobStore::new(pool.clone())
                .retain_failed_kinds(&[baukit_erasure::IDENTITY_DELETE_JOB_TYPE]),
        ),
        Arc::new(erasure_handler),
        WorkerConfig {
            queue: "identity-erasure",
            concurrency: 1,
            ..WorkerConfig::default()
        },
    )?;
    let item_service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));
    let user_service = UserService::new(Arc::new(PostgresUserRepository::new(
        pool,
        erasure_service.store().clone(),
    )));
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
    let api_state = ApiState {
        items: item_service.clone(),
{% if context.auth_oidc %}        users: user_service,
        auth: auth.clone(),
        erasure: ErasureApi {
            service: erasure_service,
            product: Arc::new(PostgresProfileErasure),
        },
{% endif %}    };
    let api = routes(api_state);
{% if context.auth_oidc %}    let rate_limit_options = RateLimitOptions::from_config(&config.rate_limit)?;
    let rate_limit_store = RedisRateLimitStore::connect_if_enabled(&rate_limit_options).await?;
    let api = if let Some(store) = rate_limit_store{% if context.mcp %}.clone(){% endif %} {
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
{% endif %}{% if context.mcp %}    let mcp_store: Arc<dyn baukit_ratelimit::RateLimitStore> = match rate_limit_store {
        Some(store) => Arc::new(store),
        None if config.environment == Environment::Local || !config.product.mcp.enabled => {
            Arc::new(baukit_ratelimit::InMemoryRateLimitStore::default())
        }
        None => {
            return Err(
                "remote MCP requires a shared rate-limit store outside local development".into(),
            );
        }
    };
    let mcp = baukit_mcp::router(
        config.product.mcp.clone(),
        Arc::new({{ context.app_crate }}_mcp::ItemTools::new(Arc::new(
            item_service.clone(),
        ))),
        mcp_store,
        {{ context.app_crate }}_mcp::authentication_policy(),
    )
    .await?;
    let api = api.merge(mcp);
{% endif %}    let api = finalize_api(api, &config.http)?;
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

{% if context.auth_oidc %}    let erasure_shutdown = shutdown.clone();
    let erasure_task = tokio::spawn(async move {
        let result = erasure_runner.run(erasure_shutdown.clone()).await;
        if result.is_err() {
            erasure_shutdown.trigger();
        }
        result
    });
{% endif %}    let signal_task = shutdown.spawn_signal_listener();
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
{% if context.auth_oidc %}    shutdown.run_during_drain(erasure_task).await???;
{% endif %}    if let Some(pool_metrics) = pool_metrics {
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
