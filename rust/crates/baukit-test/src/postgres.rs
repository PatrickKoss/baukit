use std::fmt;

#[cfg(feature = "sqlx-postgres")]
use std::path::Path;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

const POSTGRES_PORT: u16 = 5432;

/// A running disposable PostgreSQL container and its connection URL.
///
/// Keep this value alive for as long as the database is in use. Dropping it
/// invokes Testcontainers' container cleanup behavior.
pub struct PostgresTestContainer {
    connection_url: String,
    container: ContainerAsync<Postgres>,
}

impl PostgresTestContainer {
    /// Returns the host-accessible PostgreSQL connection URL.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Returns the underlying Testcontainers guard for advanced test setup.
    #[must_use]
    pub const fn container(&self) -> &ContainerAsync<Postgres> {
        &self.container
    }

    /// Splits the fixture into an owned URL and its lifetime guard.
    #[must_use]
    pub fn into_parts(self) -> (String, ContainerAsync<Postgres>) {
        (self.connection_url, self.container)
    }
}

impl fmt::Debug for PostgresTestContainer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresTestContainer")
            .field("connection_url", &self.connection_url)
            .field("container", &self.container)
            .finish()
    }
}

/// Failure while starting a PostgreSQL fixture or applying its migrations.
#[derive(Debug, thiserror::Error)]
pub enum PostgresTestError {
    /// Testcontainers could not start or inspect the PostgreSQL container.
    #[error("could not start PostgreSQL test container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// SQLx could not connect to the newly started PostgreSQL instance.
    #[cfg(feature = "sqlx-postgres")]
    #[error("could not connect to PostgreSQL test container: {0}")]
    Connect(#[source] sqlx::Error),
    /// SQLx could not load or apply the caller's migrations.
    #[cfg(feature = "sqlx-postgres")]
    #[error("could not apply PostgreSQL test migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
}

/// Starts a disposable PostgreSQL container asynchronously.
///
/// The default Testcontainers module credentials and database are all
/// `postgres`. Docker is contacted only when this function is called.
pub async fn start_postgres() -> Result<PostgresTestContainer, PostgresTestError> {
    let container = Postgres::default().start().await?;
    let host = container.get_host().await?;
    let port = container.get_host_port_ipv4(POSTGRES_PORT).await?;
    let connection_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    Ok(PostgresTestContainer {
        connection_url,
        container,
    })
}

/// Starts PostgreSQL and applies SQLx migrations from `migrations_path`.
///
/// This helper is available with the `sqlx-postgres` feature. Migration files
/// use SQLx's ordinary file naming and checksum rules.
#[cfg(feature = "sqlx-postgres")]
pub async fn start_postgres_with_migrations(
    migrations_path: impl AsRef<Path>,
) -> Result<PostgresTestContainer, PostgresTestError> {
    let fixture = start_postgres().await?;
    let migrator = sqlx::migrate::Migrator::new(migrations_path.as_ref()).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url())
        .await
        .map_err(PostgresTestError::Connect)?;
    migrator.run(&pool).await?;
    pool.close().await;
    Ok(fixture)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn starts_postgres_container() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = start_postgres().await?;
        assert!(
            fixture
                .connection_url()
                .starts_with("postgres://postgres:postgres@")
        );
        Ok(())
    }

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn starts_postgres_and_applies_migrations() -> Result<(), Box<dyn std::error::Error>> {
        use sqlx::Row as _;

        let directory = tempfile::tempdir()?;
        std::fs::write(
            directory.path().join("0001_fixture.sql"),
            "CREATE TABLE fixture (id BIGINT PRIMARY KEY);",
        )?;
        let fixture = start_postgres_with_migrations(directory.path()).await?;
        let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
        let row = sqlx::query("SELECT to_regclass('fixture')::text AS table_name")
            .fetch_one(&pool)
            .await?;
        assert_eq!(row.try_get::<String, _>("table_name")?, "fixture");
        pool.close().await;
        Ok(())
    }
}
