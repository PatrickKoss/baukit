use std::{env, error::Error, io, net::SocketAddr, sync::Arc, time::Duration};

use baukit_config::{BaukitConfig, Environment};
use baukit_jobs::{JobHandler, PostgresJobStore, WorkerConfig, WorkerRunner};
use baukit_ops::{TrafficGate, spawn_pool_metrics_sampler};
use baukit_runtime::{
    DrainBudget, ProcessKind, RestartPolicy, ServiceInfo, ShutdownToken, TaskSupervisor, build_info,
};
use baukit_telemetry::{Telemetry, TelemetryBuilder, tracing};
use sqlx::PgPool;
use tokio::net::TcpListener;

use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::config_loader;
use {{ context.app_crate }}_bin::mount_requeue_route;
use {{ context.app_crate }}_bin::worker_operations_router;
{% if context.platform or context.auth_enabled %}use {{ context.app_crate }}_worker::retention::RetentionJobHandler;
{% else %}use {{ context.app_crate }}_worker::DemoJobHandler;
{% endif %}
const PRODUCT: &str = "{{ context.app_name }}";
const POOL_METRICS_INTERVAL: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var("{{ context.app_env }}_ENVIRONMENT")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config = config_loader(environment)?.load()?;
    run(config).await
}

async fn run(config: BaukitConfig<ProductConfig>) -> Result<(), Box<dyn Error>> {
    let database = config.database.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "the durable worker requires database configuration",
        )
    })?;
    let service_info = ServiceInfo::new(
        PRODUCT,
        ProcessKind::Worker,
        build_info!(),
        config.environment,
    );
    let telemetry = init_telemetry(&config, &service_info)?;

    let job_timeout = Duration::from_secs(config.product.worker.job_timeout_seconds);
    let mut worker_database = database.clone();
    worker_database.statement_timeout = job_timeout;
    let pool = baukit_ops::connect_with_backoff(&worker_database, config.startup.budget).await?;
    let pool_metrics = spawn_pool_metrics_sampler(pool.clone(), POOL_METRICS_INTERVAL)?;
    let (runner, job_types) = build_runner(&config, &pool){% if context.platform or context.auth_enabled %}.await{% endif %}?;

    let shutdown = ShutdownToken::with_budget(DrainBudget::new(
        config.shutdown.drain_timeout,
        job_timeout,
    )?);
    let traffic_gate = TrafficGate::new();
    shutdown.on_drain({
        let traffic_gate = traffic_gate.clone();
        move || traffic_gate.stop_accepting()
    });
    let (operations, _readiness) = worker_operations_router(
        runner.clone(),
        service_info.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        traffic_gate,
    )?;
    let operations = mount_requeue_route(
        operations,
        Arc::new(job_store(&pool)),
        job_types,
        config.product.worker.admin_token.as_ref(),
    );
    let listener =
        TcpListener::bind(SocketAddr::new(config.ops.bind_address, config.ops.port)).await?;
    tracing::info!(
        message = "worker started",
        operations_address = %listener.local_addr()?,
    );

    let signal_task = shutdown.spawn_signal_listener();
    let server_result = supervise(&shutdown, runner, listener, operations).await?;
    if !signal_task.is_finished() {
        signal_task.abort();
    }
    let _signal_result = signal_task.await;
    pool.close().await;
    pool_metrics.shutdown().await;
    shutdown
        .run_flush(async move { tokio::task::spawn_blocking(move || telemetry.shutdown()).await })
        .await???;
    server_result?;
    Ok(())
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

{% if context.platform or context.auth_enabled %}async {% endif %}fn build_runner(
    config: &BaukitConfig<ProductConfig>,
    pool: &PgPool,
) -> Result<(WorkerRunner, &'static [&'static str]), Box<dyn Error>> {
{% if context.platform or context.auth_enabled %}    let handler = Arc::new(RetentionJobHandler::new(pool.clone())?);
    handler.enqueue_initial(jiff::Timestamp::now()).await?;
    let recurring = handler.recurring();
{% else %}    let handler = Arc::new(DemoJobHandler::new());
{% endif %}    let job_types = handler.job_types();
    let runner = WorkerRunner::new(
        Arc::new(job_store(pool)),
        handler,
        WorkerConfig {
            concurrency: config.product.worker.concurrency,
            poll_interval: Duration::from_millis(config.product.worker.poll_interval_milliseconds),
            job_timeout: Duration::from_secs(config.product.worker.job_timeout_seconds),
            ..WorkerConfig::default()
        },
    )?{% if context.platform or context.auth_enabled %}
    .with_recurring(recurring)?{% endif %};
    Ok((runner, job_types))
}

fn job_store(pool: &PgPool) -> PostgresJobStore {
{% if context.auth_enabled %}    PostgresJobStore::new(pool.clone())
        .retain_failed_kinds(&[baukit_erasure::IDENTITY_DELETE_JOB_TYPE])
{% else %}    PostgresJobStore::new(pool.clone())
{% endif %}}

async fn supervise(
    shutdown: &ShutdownToken,
    runner: WorkerRunner,
    listener: TcpListener,
    operations: axum::Router,
) -> Result<std::io::Result<()>, Box<dyn Error>> {
    let worker_shutdown = shutdown.child_token();
    let mut tasks = TaskSupervisor::new(shutdown.clone());
    tasks.spawn(
        "durable-job-runner",
        RestartPolicy::FailProcess,
        move || {
            let runner = runner.clone();
            let worker_shutdown = worker_shutdown.clone();
            async move {
                if let Err(error) = runner.run(worker_shutdown).await {
                    tracing::error!(%error, message = "durable worker runner failed");
                }
            }
        },
    );
    let operations_shutdown = shutdown.child_token();
    let server_result = axum::serve(
        listener,
        operations.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async move { operations_shutdown.cancelled().await })
    .await;
    shutdown.trigger();
    tasks.join().await?;
    Ok(server_result)
}
