use std::{env, error::Error, io, net::SocketAddr, path::PathBuf, sync::Arc};

use baukit_config::{BaukitConfig, ConfigLoader, Environment};
use baukit_ops::TrafficGate;
use baukit_runtime::{
    ProcessKind, RestartPolicy, ServiceInfo, ShutdownToken, TaskSupervisor, build_info,
    serve_listener_pair,
};
use baukit_telemetry::{TelemetryBuilder, tracing};
use minimal_api::{AppState, ProductConfig, api_router, openapi_document, ops_router};
use tokio::{net::TcpListener, time::Duration};

const PRODUCT: &str = "minimal-api";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let Some(arguments) = Arguments::parse()? else {
        print_usage();
        return Ok(());
    };

    if let Some(path) = arguments.openapi_out {
        baukit_openapi::write_schema(&openapi_document(), &path)?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    let config: BaukitConfig<ProductConfig> =
        ConfigLoader::new(PRODUCT, arguments.environment)?.load()?;
    run(config).await
}

async fn run(config: BaukitConfig<ProductConfig>) -> Result<(), Box<dyn Error>> {
    let service = ServiceInfo::new(PRODUCT, ProcessKind::Api, build_info!(), config.environment);
    let mut telemetry_builder = TelemetryBuilder::new(service.telemetry_identity().clone())
        .sampling_ratio(config.telemetry.trace_sampling_ratio)
        .log_format(config.telemetry.log_format);
    if let Some(endpoint) = &config.telemetry.otlp_endpoint {
        telemetry_builder = telemetry_builder.otlp_endpoint(endpoint);
    }
    let telemetry = Arc::new(telemetry_builder.init()?);

    let state = AppState::new(config.product.max_notes);
    let shutdown = ShutdownToken::new(config.shutdown.drain_timeout);
    let traffic_gate = TrafficGate::new();
    shutdown.on_drain({
        let traffic_gate = traffic_gate.clone();
        move || traffic_gate.stop_accepting()
    });
    let api = api_router(state.clone(), &config.http)?;
    let operations = ops_router(
        state.clone(),
        service.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        traffic_gate.clone(),
    )?;

    let api_listener =
        TcpListener::bind(SocketAddr::new(config.http.bind_address, config.http.port)).await?;
    let operations_listener =
        TcpListener::bind(SocketAddr::new(config.ops.bind_address, config.ops.port)).await?;
    tracing::info!(
        message = "service started",
        api_address = %api_listener.local_addr()?,
        operations_address = %operations_listener.local_addr()?,
        service = service.name(),
    );

    let signal_task = shutdown.spawn_signal_listener();

    let mut supervisor = TaskSupervisor::new(shutdown.clone());
    let janitor_shutdown = shutdown.child_token();
    supervisor.spawn(
        "note-count-janitor",
        RestartPolicy::FailProcess,
        move || note_count_janitor(state.clone(), janitor_shutdown.clone()),
    );

    let listener_result = serve_listener_pair(
        api_listener,
        api,
        operations_listener,
        operations,
        shutdown.clone(),
    )
    .await;

    shutdown.trigger();
    supervisor.join().await?;
    if !signal_task.is_finished() {
        signal_task.abort();
    }
    let _signal_result = signal_task.await;

    let telemetry_for_flush = Arc::clone(&telemetry);
    shutdown
        .run_during_drain(async move {
            tokio::task::spawn_blocking(move || telemetry_for_flush.shutdown()).await
        })
        .await???;
    tracing::info!(message = "service stopped");
    listener_result?;
    Ok(())
}

async fn note_count_janitor(state: AppState, shutdown: ShutdownToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => break,
            _ = interval.tick() => match state.note_count() {
                Ok(note_count) => tracing::info!(message = "note janitor inspected state", note_count),
                Err(error) => tracing::error!(message = "note janitor could not inspect state", %error),
            },
        }
    }
}

struct Arguments {
    environment: Environment,
    openapi_out: Option<PathBuf>,
}

impl Arguments {
    fn parse() -> Result<Option<Self>, Box<dyn Error>> {
        let mut environment = env::var("MINIMAL_API_ENVIRONMENT")
            .ok()
            .map(|value| value.parse())
            .transpose()?
            .unwrap_or_default();
        let mut openapi_out = env::var_os("OPENAPI_OUT").map(PathBuf::from);
        let mut arguments = env::args().skip(1);

        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--openapi" => {
                    openapi_out.get_or_insert_with(|| PathBuf::from("openapi.json"));
                }
                "--environment" => {
                    let value = arguments.next().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "--environment requires local, staging, or production",
                        )
                    })?;
                    environment = value.parse()?;
                }
                "--help" | "-h" => return Ok(None),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("unknown argument `{argument}`"),
                    )
                    .into());
                }
            }
        }

        Ok(Some(Self {
            environment,
            openapi_out,
        }))
    }
}

fn print_usage() {
    println!(
        "Usage: minimal-api [--environment local|staging|production] [--openapi]\n\
         MINIMAL_API_ENVIRONMENT selects the deployment environment.\n\
         OPENAPI_OUT writes the schema to that path and exits."
    );
}
