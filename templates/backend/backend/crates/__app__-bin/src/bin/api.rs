use std::{env, error::Error, net::SocketAddr, sync::Arc, time::Duration};

use axum::Router;
{% if context.auth_enabled %}use baukit_auth::AuthState;
{% endif %}use baukit_config::{BaukitConfig, Environment};
{% if context.auth_enabled %}use baukit_jobs::{PostgresJobStore, WorkerConfig, WorkerRunner};
{% endif %}use baukit_ops::{PoolMetricsSampler, TrafficGate, spawn_pool_metrics_sampler};
use baukit_runtime::{ProcessKind, ServiceInfo, ShutdownToken, build_info, serve_listener_pair};
use baukit_telemetry::{Telemetry, TelemetryBuilder, tracing};

use sqlx::{PgPool, postgres::PgPoolOptions};
use tokio::net::TcpListener;

{% if context.auth_enabled %}use {{ context.app_crate }}_api::ApiState;
use {{ context.app_crate }}_api::ErasureApi;
{% endif %}use {{ context.app_crate }}_bin::ApiDependencies;
{% if not context.auth_enabled and not context.platform %}use {{ context.app_crate }}_bin::InMemoryItemRepository;
{% endif %}use {{ context.app_crate }}_bin::ProductConfig;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::auth_verifier;
{% endif %}use {{ context.app_crate }}_bin::compose_api;
use {{ context.app_crate }}_bin::config_loader;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::identity_erasure;
{% endif %}use {{ context.app_crate }}_bin::operations_router;
{% if not context.auth_enabled and not context.platform %}use {{ context.app_crate }}_ports::ItemRepository;
{% endif %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% if context.auth_enabled %}use {{ context.app_crate }}_postgres::PostgresProfileErasure;
use {{ context.app_crate }}_postgres::PostgresUserRepository;
{% endif %}use {{ context.app_crate }}_services::ItemService;
{% if context.auth_enabled %}use {{ context.app_crate }}_services::UserService;
{% endif %}
const PRODUCT: &str = "{{ context.app_name }}";
const ENV_PREFIX: &str = "{{ context.app_env }}";
const POOL_METRICS_INTERVAL: Duration = Duration::from_secs(15);

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
    let api = compose_api(&config, &backend.api){% if context.auth_enabled %}.await{% endif %}?.router;
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
    api: ApiDependencies,
    pool_metrics: Option<PoolMetricsSampler>,
{% if context.auth_enabled %}    erasure_runner: WorkerRunner,
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
            api: ApiDependencies {
                pool,
                item_service,
                api_state,
                auth,
{% if context.mcp %}                verifier,
{% endif %}                rate_limit_store: None,
            },
            pool_metrics: Some(pool_metrics),
            erasure_runner,
        })
    }
{% elif context.platform %}    async fn connect(config: &BaukitConfig<ProductConfig>) -> Result<Self, Box<dyn Error>> {
        let (pool, pool_metrics) = connect_pool(config)
            .await?
            .ok_or("platform backend requires database configuration")?;
        let repository = Arc::new(PostgresItemRepository::new(pool.clone()));
        Ok(Self {
            api: ApiDependencies {
                item_service: ItemService::new(repository),
                pool,
                rate_limit_store: None,
            },
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
            api: ApiDependencies {
                item_service: ItemService::new(repository),
                rate_limit_store: None,
            },
            pool_metrics,
        })
    }
{% endif %}}

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
        backend.api.item_service,
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
