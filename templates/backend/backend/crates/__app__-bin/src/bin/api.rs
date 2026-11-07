use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

use axum::Router;
{% if context.auth_enabled %}use baukit_auth::AuthState;
{% endif %}{% if context.mcp %}use baukit_auth::IdentityVerifier;
{% endif %}use baukit_config::{BaukitConfig, Environment};
{% if context.auth_enabled %}use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
{% endif %}use baukit_ops::{PoolMetricsSampler, TrafficGate, spawn_pool_metrics_sampler};
{% if context.auth_enabled %}use baukit_ratelimit::{RateLimitOptions, RedisRateLimitStore, SharedRateLimitStore};
{% else %}use baukit_ratelimit::{InMemoryRateLimitStore, RateLimitOptions, SharedRateLimitStore};
{% endif %}use baukit_runtime::{ProcessKind, ServiceInfo, ShutdownToken, build_info, serve_listener_pair};
use baukit_telemetry::{Telemetry, TelemetryBuilder, tracing};

use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::net::TcpListener;

use {{ context.app_crate }}_api::ApiState;
{% if context.auth_enabled %}use {{ context.app_crate }}_api::ErasureApi;
use {{ context.app_crate }}_api::finalize_with_options;
use {{ context.app_crate }}_api::http_options;
{% else %}use {{ context.app_crate }}_api::finalize_api;
{% endif %}use {{ context.app_crate }}_api::routes;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::BrowserSessionDependencies;
{% endif %}{% if not context.auth_enabled and not context.platform %}use {{ context.app_crate }}_bin::InMemoryItemRepository;
{% endif %}use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::RateLimits;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::auth_verifier;
use {{ context.app_crate }}_bin::browser_session_router;
{% endif %}use {{ context.app_crate }}_bin::config_loader;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::identity_erasure;
{% endif %}{% if context.mcp %}use {{ context.app_crate }}_bin::mcp_policy;
use {{ context.app_crate }}_bin::mcp_verifier;
{% endif %}use {{ context.app_crate }}_bin::operations_router;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::protect_routes;
{% endif %}{% if not context.auth_enabled or context.platform %}use {{ context.app_crate }}_bin::rate_limited;
{% endif %}{% if not context.auth_enabled and not context.platform %}use {{ context.app_crate }}_ports::ItemRepository;
{% endif %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% if context.auth_enabled %}use {{ context.app_crate }}_postgres::PostgresProfileErasure;
use {{ context.app_crate }}_postgres::PostgresUserRepository;
{% endif %}use {{ context.app_crate }}_services::ItemService;
{% if context.auth_enabled %}use {{ context.app_crate }}_services::UserService;
{% endif %}
const PRODUCT: &str = "{{ context.app_name }}";
const ENV_PREFIX: &str = "{{ context.app_env }}";
const POOL_METRICS_INTERVAL: Duration = Duration::from_secs(15);
{% if context.auth_enabled %}const HSTS_MAX_AGE: Duration = Duration::from_secs(365 * 24 * 60 * 60);
{% endif %}
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var(format!("{ENV_PREFIX}_ENVIRONMENT"))
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config = config_loader(environment)?.load()?;
    run(config).await
}

async fn run(config: BaukitConfig<ProductConfig>) -> Result<(), Box<dyn Error>> {
    let service_info =
        ServiceInfo::new(PRODUCT, ProcessKind::Api, build_info!(), config.environment);
    let telemetry = init_telemetry(&config, &service_info)?;
    let backend = Backend::connect(&config).await?;
    let api = compose_api(&config, &backend){% if context.auth_enabled %}.await{% endif %}?;
    serve(&config, &service_info, telemetry, backend, api).await
}

fn init_telemetry(
    config: &BaukitConfig<ProductConfig>,
    service_info: &ServiceInfo,
) -> Result<Arc<Telemetry>, Box<dyn Error>> {
    let mut builder = TelemetryBuilder::new(service_info.telemetry_identity().clone())
        .sampling_ratio(config.telemetry.trace_sampling_ratio)
        .log_format(config.telemetry.log_format);
    if let Some(endpoint) = &config.telemetry.otlp_endpoint {
        builder = builder.otlp_endpoint(endpoint);
    }
    Ok(Arc::new(builder.init()?))
}

async fn connect_pool(
    config: &BaukitConfig<ProductConfig>,
) -> Result<Option<(PgPool, PoolMetricsSampler)>, Box<dyn Error>> {
    let Some(database) = &config.database else {
        return Ok(None);
    };
    let pool = PgPoolOptions::new()
        .max_connections(database.max_connections)
        .min_connections(database.min_connections)
        .acquire_timeout(database.acquire_timeout)
        .connect(database.url.expose())
        .await?;
    let pool_metrics = spawn_pool_metrics_sampler(pool.clone(), POOL_METRICS_INTERVAL)?;
    Ok(Some((pool, pool_metrics)))
}

struct Backend {
{% if context.auth_enabled or context.platform %}    pool: PgPool,
{% endif %}    pool_metrics: Option<PoolMetricsSampler>,
    item_service: ItemService,
{% if context.auth_enabled %}    api_state: ApiState,
    auth: AuthState,
{% if context.mcp %}    verifier: Arc<dyn IdentityVerifier>,
{% endif %}    erasure_runner: WorkerRunner,
{% endif %}}

impl Backend {
{% if context.auth_enabled %}    async fn connect(config: &BaukitConfig<ProductConfig>) -> Result<Self, Box<dyn Error>> {
        let (pool, pool_metrics) = connect_pool(config)
            .await?
            .ok_or("authenticated backend requires database configuration")?;
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
        let users = UserService::new(Arc::new(PostgresUserRepository::new(
            pool.clone(),
            erasure_service.store().clone(),
        )));
        let verifier = auth_verifier(&config.product.auth).await?;
        let auth = AuthState::from_shared(verifier{% if context.mcp %}.clone(){% endif %});
        let api_state = ApiState {
            items: item_service.clone(),
            users,
            auth: auth.clone(),
            erasure: ErasureApi {
                service: erasure_service,
                product: Arc::new(PostgresProfileErasure),
            },
        };
        Ok(Self {
            pool,
            pool_metrics: Some(pool_metrics),
            item_service,
            api_state,
            auth,
{% if context.mcp %}            verifier,
{% endif %}            erasure_runner,
        })
    }
{% elif context.platform %}    async fn connect(config: &BaukitConfig<ProductConfig>) -> Result<Self, Box<dyn Error>> {
        let (pool, pool_metrics) = connect_pool(config)
            .await?
            .ok_or("platform backend requires database configuration")?;
        Ok(Self {
            item_service: ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone()))),
            pool,
            pool_metrics: Some(pool_metrics),
        })
    }
{% else %}    async fn connect(config: &BaukitConfig<ProductConfig>) -> Result<Self, Box<dyn Error>> {
        let (repository, pool_metrics): (Arc<dyn ItemRepository>, Option<PoolMetricsSampler>) =
            match connect_pool(config).await? {
                Some((pool, pool_metrics)) => (
                    Arc::new(PostgresItemRepository::new(pool)),
                    Some(pool_metrics),
                ),
                None => {
                    tracing::warn!(
                        message = "database is not configured; using the in-memory item adapter"
                    );
                    (Arc::new(InMemoryItemRepository::new()), None)
                }
            };
        Ok(Self {
            pool_metrics,
            item_service: ItemService::new(repository),
        })
    }
{% endif %}}
{% if context.auth_enabled %}
async fn compose_api(
    config: &BaukitConfig<ProductConfig>,
    backend: &Backend,
) -> Result<Router, Box<dyn Error>> {
    let rate_limit_options = RateLimitOptions::from_config(&config.rate_limit)?;
    let redis = RedisRateLimitStore::connect_if_enabled(&rate_limit_options).await?;
    let limits = redis.clone().map(|store| RateLimits {
        store: SharedRateLimitStore::new(store),
        options: rate_limit_options.clone(),
    });
    let mut http = http_options(&config.http)?;
    if config.environment != Environment::Local {
        http = http.with_hsts(HSTS_MAX_AGE);
    }
    let mut late = Vec::new();
    if config.product.auth.browser_session.enabled {
        http.allow_credentials = true;
        let dependencies = BrowserSessionDependencies {
            issuer: &config.product.auth.issuer,
            http: &http,
            pool: backend.pool.clone(),
            rate_limit: &rate_limit_options,
            rate_limit_store: redis,
            environment: config.environment,
        };
        late.extend(
            browser_session_router(&config.product.auth.browser_session, dependencies).await?,
        );
    }
{% if context.platform %}    let pool = backend.pool.clone();
    let mut platform = {{ context.app_crate }}_bin::platform::compose(pool, &config.product)?;
    platform.router = rate_limited(platform.router, limits.as_ref());
{% endif %}    let api = protect_routes(
        routes(backend.api_state.clone()),
        late,
        backend.auth.clone(),
        limits.as_ref(),
    )?;
{% if context.mcp %}    // baukit_mcp limits its own routes with the same store, keyed by the verified MCP caller.
    let mcp_store = mcp_store(config, limits.as_ref())?;
    let item_reads = Arc::new(backend.item_service.clone());
    let mcp_services = {{ context.app_crate }}_mcp::services(item_reads);
    let mcp = baukit_mcp::router(
        config.product.mcp.clone(),
        mcp_caller_verifier(config, backend).await?,
        mcp_services,
        mcp_store,
        mcp_policy(&config.product.auth, &config.product.mcp)?,
    )
    .await?;
    let api = api.merge(mcp);
{% endif %}{% if context.platform %}    let api = api
        .merge(platform.router)
        .layer(axum::Extension(platform.entitlements));
{% endif %}    Ok(finalize_with_options(api, http))
}
{% if context.mcp %}
fn mcp_store(
    config: &BaukitConfig<ProductConfig>,
    limits: Option<&RateLimits>,
) -> Result<Arc<dyn baukit_ratelimit::RateLimitStore>, Box<dyn Error>> {
    match limits {
        Some(limits) => Ok(Arc::new(limits.store.clone())),
        None if config.environment == Environment::Local || !config.product.mcp.enabled => {
            Ok(Arc::new(baukit_ratelimit::InMemoryRateLimitStore::default()))
        }
        None => {
            Err("remote MCP requires a shared rate-limit store outside local development".into())
        }
    }
}

async fn mcp_caller_verifier(
    config: &BaukitConfig<ProductConfig>,
    backend: &Backend,
) -> Result<Arc<dyn IdentityVerifier>, Box<dyn Error>> {
    if config.product.mcp.enabled {
        return mcp_verifier(&config.product.auth, &config.product.mcp).await;
    }
    Ok(backend.verifier.clone())
}
{% endif %}{% else %}
fn compose_api(
    config: &BaukitConfig<ProductConfig>,
    backend: &Backend,
) -> Result<Router, Box<dyn Error>> {
    let options = RateLimitOptions::from_config(&config.rate_limit)?;
    // Without auth there is no Redis in the stack, so each replica keeps its own IP buckets.
    let limits = options.is_enabled().then(|| RateLimits {
        store: SharedRateLimitStore::new(InMemoryRateLimitStore::default()),
        options,
    });
    let api = routes(ApiState {
        items: backend.item_service.clone(),
    });
{% if context.platform %}    let pool = backend.pool.clone();
    let platform = {{ context.app_crate }}_bin::platform::compose(pool, &config.product)?;
    let api = api.merge(platform.router);
{% endif %}    let api = rate_limited(api, limits.as_ref());
{% if context.platform %}    let api = api.layer(axum::Extension(platform.entitlements));
{% endif %}    Ok(finalize_api(api, &config.http)?)
}
{% endif %}
async fn serve(
    config: &BaukitConfig<ProductConfig>,
    service_info: &ServiceInfo,
    telemetry: Arc<Telemetry>,
    backend: Backend,
    api: Router,
) -> Result<(), Box<dyn Error>> {
    let shutdown = ShutdownToken::new(config.shutdown.drain_timeout);
    let traffic_gate = TrafficGate::new();
    shutdown.on_drain({
        let traffic_gate = traffic_gate.clone();
        move || traffic_gate.stop_accepting()
    });
    let (operations, _readiness) = operations_router(
        backend.item_service,
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
{% if context.auth_enabled %}    let erasure_shutdown = shutdown.clone();
    let erasure_runner = backend.erasure_runner;
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
{% if context.auth_enabled %}    shutdown.run_during_drain(erasure_task).await???;
{% endif %}    if let Some(pool_metrics) = backend.pool_metrics {
        pool_metrics.shutdown().await;
    }
    shutdown
        .run_during_drain(
            async move { tokio::task::spawn_blocking(move || telemetry.shutdown()).await },
        )
        .await???;
    result?;
    Ok(())
}
