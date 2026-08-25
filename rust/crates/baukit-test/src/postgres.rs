use std::fmt;

#[cfg(feature = "sqlx-postgres")]
use sqlx::Row as _;
#[cfg(feature = "sqlx-postgres")]
use std::path::Path;
use testcontainers::{ContainerAsync, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

#[cfg(feature = "sqlx-postgres")]
use crate::{CleanupKind, OwnedResourceCheck};

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

/// Direct foreign key whose delete action conflicts with the resource registry.
#[cfg(feature = "sqlx-postgres")]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ForeignKeyDeleteMismatch {
    /// PostgreSQL constraint name.
    pub constraint_name: String,
    /// Schema-qualified table that owns the foreign key.
    pub referencing_table: String,
    /// Delete action reported by PostgreSQL, such as `NO ACTION`.
    pub actual_delete_action: String,
    /// Cleanup kind required by the registry, or cascade for an unregistered table.
    pub declared_cleanup: CleanupKind,
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

/// Lists direct foreign keys to `user_root_table` that violate declared cleanup.
///
/// Registry names may be unqualified table names or schema-qualified names.
/// Unregistered direct references must use `ON DELETE CASCADE`.
#[cfg(feature = "sqlx-postgres")]
pub async fn audit_user_root_foreign_keys(
    pool: &sqlx::PgPool,
    user_root_table: &str,
    resources: &[OwnedResourceCheck],
) -> Result<Vec<ForeignKeyDeleteMismatch>, sqlx::Error> {
    let root = sqlx::query("SELECT to_regclass($1)::text AS table_name")
        .bind(user_root_table)
        .fetch_one(pool)
        .await?
        .try_get::<Option<String>, _>("table_name")?;
    if root.is_none() {
        return Err(sqlx::Error::RowNotFound);
    }

    let rows = sqlx::query(
        "SELECT constraint_row.conname AS constraint_name, \
                child.relname AS table_name, \
                quote_ident(child_namespace.nspname) || '.' || quote_ident(child.relname) AS qualified_table, \
                CASE constraint_row.confdeltype \
                    WHEN 'a' THEN 'NO ACTION' \
                    WHEN 'r' THEN 'RESTRICT' \
                    WHEN 'c' THEN 'CASCADE' \
                    WHEN 'n' THEN 'SET NULL' \
                    WHEN 'd' THEN 'SET DEFAULT' \
                    ELSE 'UNKNOWN' \
                END AS delete_action \
         FROM pg_constraint AS constraint_row \
         JOIN pg_class AS child ON child.oid = constraint_row.conrelid \
         JOIN pg_namespace AS child_namespace ON child_namespace.oid = child.relnamespace \
         WHERE constraint_row.contype = 'f' \
           AND constraint_row.confrelid = to_regclass($1) \
         ORDER BY qualified_table, constraint_name",
    )
    .bind(user_root_table)
    .fetch_all(pool)
    .await?;

    let mut mismatches = Vec::new();
    for row in rows {
        let table_name = row.try_get::<String, _>("table_name")?;
        let qualified_table = row.try_get::<String, _>("qualified_table")?;
        let declared_cleanup = resources
            .iter()
            .find(|resource| resource.name == table_name || resource.name == qualified_table)
            .map_or(CleanupKind::Cascade, |resource| resource.cleanup);
        let actual_delete_action = row.try_get::<String, _>("delete_action")?;
        if declared_cleanup == CleanupKind::Cascade && actual_delete_action != "CASCADE" {
            mismatches.push(ForeignKeyDeleteMismatch {
                constraint_name: row.try_get("constraint_name")?,
                referencing_table: qualified_table,
                actual_delete_action,
                declared_cleanup,
            });
        }
    }
    Ok(mismatches)
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

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn audits_direct_user_root_foreign_keys() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = start_postgres().await?;
        let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
        sqlx::query("CREATE TABLE erasure_users (id BIGINT PRIMARY KEY)")
            .execute(&pool)
            .await?;
        sqlx::query(
            "CREATE TABLE cascading_records ( \
                 id BIGINT PRIMARY KEY, \
                 user_id BIGINT NOT NULL REFERENCES erasure_users(id) ON DELETE CASCADE \
             )",
        )
        .execute(&pool)
        .await?;
        sqlx::query(
            "CREATE TABLE restricted_records ( \
                 id BIGINT PRIMARY KEY, \
                 user_id BIGINT NOT NULL REFERENCES erasure_users(id) \
             )",
        )
        .execute(&pool)
        .await?;
        let registry = [
            OwnedResourceCheck {
                name: "cascading_records",
                count_sql: "SELECT count(*) FROM cascading_records WHERE user_id = $1",
                cleanup: CleanupKind::Cascade,
            },
            OwnedResourceCheck {
                name: "restricted_records",
                count_sql: "SELECT count(*) FROM restricted_records WHERE user_id = $1",
                cleanup: CleanupKind::Cascade,
            },
        ];

        let mismatches = audit_user_root_foreign_keys(&pool, "erasure_users", &registry).await?;

        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].referencing_table, "public.restricted_records");
        assert_eq!(mismatches[0].actual_delete_action, "NO ACTION");
        for cleanup in [CleanupKind::Explicit, CleanupKind::AsyncProcessor] {
            let non_cascade_registry = [
                registry[0],
                OwnedResourceCheck {
                    cleanup,
                    ..registry[1]
                },
            ];
            assert!(
                audit_user_root_foreign_keys(&pool, "erasure_users", &non_cascade_registry)
                    .await?
                    .is_empty()
            );
        }
        pool.close().await;
        Ok(())
    }
}
