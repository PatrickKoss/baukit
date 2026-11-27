#[path = "support/service_process.rs"]
mod service_process;

use std::{error::Error, path::PathBuf, time::Duration};

use baukit_config::ShutdownConfig;

use service_process::{ServiceProcess, config_env, free_loopback_address};

const EXIT_MARGIN: Duration = Duration::from_secs(10);

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for durable worker verification"]
async fn worker_binary_starts_with_defaults_and_drains_on_sigterm() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_owner_migrations(
        &migrations,
        baukit_test::PostgresMigrationOwner::default(){% if context.postgres_owner_create_role %}.with_create_role(){% endif %},
    )
    .await?;
    let database_url = fixture.owner_connection_url().ok_or("missing owner URL")?;
    let ops = free_loopback_address()?;
    let workdir = tempfile::tempdir()?;
    let env = [
        (config_env("DATABASE__URL"), database_url.to_owned()),
        (config_env("OPS__BIND_ADDRESS"), ops.ip().to_string()),
        (config_env("OPS__PORT"), ops.port().to_string()),
    ];

    let mut worker = ServiceProcess::spawn(env!("CARGO_BIN_EXE_worker"), workdir.path(), &env)?;
    worker.wait_until_ready(ops).await?;
    worker.terminate()?;
    let drain_timeout = ShutdownConfig::default().drain_timeout;
    let status = worker.wait_for_exit(drain_timeout + EXIT_MARGIN).await?;
    if !status.success() {
        return Err(worker.failure(&format!("exited with {status} after SIGTERM")));
    }
    drop(fixture);
    Ok(())
}
