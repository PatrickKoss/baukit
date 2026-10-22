use std::{env, error::Error};

use baukit_config::Environment;
use {{ context.app_crate }}_bin::{ProductConfig, config_loader, platform::purge_receipts};
use sqlx::postgres::PgPoolOptions;

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
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database.url.expose())
        .await?;
    let removed = purge_receipts(pool.clone()).await?;
    pool.close().await;
    println!("purged {removed} platform receipts");
    Ok(())
}
