use std::{env, error::Error, io, path::PathBuf};

use baukit_config::{BaukitConfig, Environment};

use {{ context.app_crate }}_bin::{ProductConfig, config_loader};

const ENV_PREFIX: &str = "{{ context.app_env }}";

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let environment = env::var(format!("{ENV_PREFIX}_ENVIRONMENT"))
        .ok()
        .map(|value| value.parse())
        .transpose()?
        .unwrap_or(Environment::Local);
    let config: BaukitConfig<ProductConfig> = config_loader(environment)?.load()?;
    let database = config.database.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "database configuration is required; set {{ context.app_env }}__DATABASE__URL",
        )
    })?;
    let pool = sqlx::PgPool::connect(database.url.expose()).await?;
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let migrator = sqlx::migrate::Migrator::new(migrations.clone()).await?;
    migrator.run(&pool).await?;
    pool.close().await;
    println!("applied migrations from {}", migrations.display());
    Ok(())
}
