use std::{env, error::Error, io::Write as _};

use baukit_config::Environment;
use {{ context.app_crate }}_bin::{ProductConfig, config_loader, platform::purge_receipts};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var("{{ context.app_env }}_ENVIRONMENT")
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config = config_loader(environment)?.load::<ProductConfig>()?;
    let database = config
        .database
        .as_ref()
        .ok_or("receipt purge requires database configuration")?;
    let mut purge_database = database.clone();
    purge_database.max_connections = 1;
    purge_database.min_connections = 0;
    let pool = baukit_ops::connect_with_backoff(&purge_database, config.startup.budget).await?;
    let removed = purge_receipts(pool.clone()).await?;
    pool.close().await;
    writeln!(
        std::io::stdout().lock(),
        "purged {removed} platform receipts"
    )?;
    Ok(())
}
