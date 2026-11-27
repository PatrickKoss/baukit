#[path = "support/service_process.rs"]
mod service_process;

use std::{error::Error, path::PathBuf, time::Duration};

use baukit_config::ShutdownConfig;

use service_process::{ServiceProcess, config_env, free_loopback_address};

const EXIT_MARGIN: Duration = Duration::from_secs(10);

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for PostgreSQL verification"]
async fn api_binary_starts_with_defaults_and_drains_on_sigterm() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_owner_migrations(
        &migrations,
        baukit_test::PostgresMigrationOwner::default(){% if context.postgres_owner_create_role %}.with_create_role(){% endif %},
    )
    .await?;
    let database_url = fixture.owner_connection_url().ok_or("missing owner URL")?;
    let http = free_loopback_address()?;
    let ops = free_loopback_address()?;
    let workdir = tempfile::tempdir()?;
    let env = [
        (config_env("DATABASE__URL"), database_url.to_owned()),
        (config_env("HTTP__BIND_ADDRESS"), http.ip().to_string()),
        (config_env("HTTP__PORT"), http.port().to_string()),
        (config_env("OPS__BIND_ADDRESS"), ops.ip().to_string()),
        (config_env("OPS__PORT"), ops.port().to_string()),
    ];

    let mut api = ServiceProcess::spawn(env!("CARGO_BIN_EXE_api"), workdir.path(), &env)?;
    api.wait_until_ready(ops).await?;
    api.terminate()?;
    let drain_timeout = ShutdownConfig::default().drain_timeout;
    let status = api.wait_for_exit(drain_timeout + EXIT_MARGIN).await?;
    if !status.success() {
        return Err(api.failure(&format!("exited with {status} after SIGTERM")));
    }
    drop(fixture);
    Ok(())
}
