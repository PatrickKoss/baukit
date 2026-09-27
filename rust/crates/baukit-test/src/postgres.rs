use std::fmt;

#[cfg(feature = "sqlx-postgres")]
use sqlx::Row as _;
#[cfg(feature = "sqlx-postgres")]
use std::path::{Path, PathBuf};
use testcontainers::{ContainerAsync, ImageExt as _, runners::AsyncRunner};
use testcontainers_modules::postgres::Postgres;

#[cfg(feature = "sqlx-postgres")]
use crate::postgres_database::{PostgresAppRole, PostgresTestDatabase, PostgresTestDatabases};
#[cfg(feature = "sqlx-postgres")]
use crate::{CleanupKind, OwnedResourceCheck};

const POSTGRES_IMAGE_NAME: &str = "postgres";
const POSTGRES_IMAGE_TAG: &str = "18-alpine";
const POSTGRES_PORT: u16 = 5432;

/// A running disposable PostgreSQL container and its connection URL.
///
/// Keep this value alive for as long as the database is in use. Dropping it
/// invokes Testcontainers' container cleanup behavior.
pub struct PostgresTestContainer {
    connection_url: String,
    #[cfg(feature = "sqlx-postgres")]
    app_connection_url: Option<String>,
    #[cfg(feature = "sqlx-postgres")]
    databases: PostgresTestDatabases,
    container: ContainerAsync<Postgres>,
}

impl PostgresTestContainer {
    /// Returns the host-accessible PostgreSQL connection URL.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Returns the URL for the configured app role on the `postgres` database.
    ///
    /// `None` unless [`PostgresTestOptions::with_app_role`] was set.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn app_connection_url(&self) -> Option<&str> {
        self.app_connection_url.as_deref()
    }

    /// Creates a separate database on this container for one test.
    ///
    /// The database gets the container's app role grants and migrations. It is
    /// dropped when the returned value drops.
    #[cfg(feature = "sqlx-postgres")]
    pub async fn create_database(&self) -> Result<PostgresTestDatabase, PostgresTestError> {
        self.databases.create().await
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
            .finish_non_exhaustive()
    }
}

/// Image, app role, and migrations for a disposable PostgreSQL container.
///
/// The default is the official `postgres:18-alpine` image with only the
/// `postgres` superuser, which is what [`start_postgres`] starts.
#[derive(Clone, Debug)]
pub struct PostgresTestOptions {
    image_name: String,
    image_tag: String,
    #[cfg(feature = "sqlx-postgres")]
    app_role: Option<PostgresAppRole>,
    #[cfg(feature = "sqlx-postgres")]
    migrations: Option<PathBuf>,
}

impl Default for PostgresTestOptions {
    fn default() -> Self {
        Self {
            image_name: POSTGRES_IMAGE_NAME.to_owned(),
            image_tag: POSTGRES_IMAGE_TAG.to_owned(),
            #[cfg(feature = "sqlx-postgres")]
            app_role: None,
            #[cfg(feature = "sqlx-postgres")]
            migrations: None,
        }
    }
}

impl PostgresTestOptions {
    /// Creates the default options.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the image, for example `timescale/timescaledb` and `latest-pg17`.
    ///
    /// The image must accept the official image's `POSTGRES_USER`,
    /// `POSTGRES_PASSWORD`, and `POSTGRES_DB` variables.
    #[must_use]
    pub fn with_image(mut self, name: impl Into<String>, tag: impl Into<String>) -> Self {
        self.image_name = name.into();
        self.image_tag = tag.into();
        self
    }

    /// Creates a login role without superuser or `BYPASSRLS` before migrations run.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn with_app_role(mut self, role: PostgresAppRole) -> Self {
        self.app_role = Some(role);
        self
    }

    /// Applies SQLx migrations from `path` as the superuser after the app role exists.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn with_migrations(mut self, path: impl Into<PathBuf>) -> Self {
        self.migrations = Some(path.into());
        self
    }

    /// Starts the container and, with `sqlx-postgres`, waits until it accepts
    /// queries, then prepares the app role and migrations.
    ///
    /// Docker is contacted only when this function is called.
    pub async fn start(self) -> Result<PostgresTestContainer, PostgresTestError> {
        let container = Postgres::default()
            .with_name(self.image_name)
            .with_tag(self.image_tag)
            .start()
            .await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(POSTGRES_PORT).await?;
        let connection_url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        #[cfg(feature = "sqlx-postgres")]
        {
            let mut databases = PostgresTestDatabases::new(connection_url.clone());
            if let Some(role) = self.app_role {
                databases = databases.with_app_role(role);
            }
            if let Some(path) = self.migrations {
                databases = databases.with_migrations(path);
            }
            let app_connection_url = databases.prepare_server_database().await?;
            Ok(PostgresTestContainer {
                connection_url,
                app_connection_url,
                databases,
                container,
            })
        }
        #[cfg(not(feature = "sqlx-postgres"))]
        Ok(PostgresTestContainer {
            connection_url,
            container,
        })
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
    /// An app role name or password is not accepted, or an existing role bypasses RLS.
    #[cfg(feature = "sqlx-postgres")]
    #[error("invalid PostgreSQL test app role: {0}")]
    InvalidAppRole(&'static str),
    /// The server URL could not be parsed or rewritten for another database.
    #[cfg(feature = "sqlx-postgres")]
    #[error("invalid PostgreSQL test connection URL")]
    InvalidConnectionUrl,
    /// Creating, preparing, or dropping a test database or role failed.
    #[cfg(feature = "sqlx-postgres")]
    #[error("could not prepare PostgreSQL test database: {0}")]
    Setup(#[source] sqlx::Error),
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

/// Starts a disposable PostgreSQL 18 container asynchronously.
///
/// The default Testcontainers module credentials and database are all
/// `postgres`. Docker is contacted only when this function is called.
pub async fn start_postgres() -> Result<PostgresTestContainer, PostgresTestError> {
    PostgresTestOptions::new().start().await
}

/// Starts PostgreSQL 18 and applies SQLx migrations from `migrations_path`.
///
/// This helper is available with the `sqlx-postgres` feature. Migration files
/// use SQLx's ordinary file naming and checksum rules.
#[cfg(feature = "sqlx-postgres")]
pub async fn start_postgres_with_migrations(
    migrations_path: impl AsRef<Path>,
) -> Result<PostgresTestContainer, PostgresTestError> {
    PostgresTestOptions::new()
        .with_migrations(migrations_path.as_ref())
        .start()
        .await
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
    #[cfg(feature = "sqlx-postgres")]
    use sqlx::Connection as _;

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
        let version = sqlx::query("SELECT current_setting('server_version_num') AS version")
            .fetch_one(&pool)
            .await?
            .try_get::<String, _>("version")?;
        assert!(
            version.starts_with("18"),
            "unexpected PostgreSQL version: {version}"
        );
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
    async fn starts_an_overridden_image_tag() -> Result<(), Box<dyn std::error::Error>> {
        let fixture = PostgresTestOptions::new()
            .with_image("postgres", "17-alpine")
            .start()
            .await?;
        let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
        let version = sqlx::query("SELECT current_setting('server_version_num') AS version")
            .fetch_one(&pool)
            .await?
            .try_get::<String, _>("version")?;
        assert!(version.starts_with("17"), "unexpected version: {version}");
        pool.close().await;
        Ok(())
    }

    #[cfg(feature = "sqlx-postgres")]
    const RLS_MIGRATION: &str = "\
        CREATE TABLE notes (owner TEXT NOT NULL, body TEXT NOT NULL); \
        ALTER TABLE notes ENABLE ROW LEVEL SECURITY; \
        CREATE POLICY notes_owner ON notes \
            USING (owner = current_setting('app.owner', true)) \
            WITH CHECK (owner = current_setting('app.owner', true));";

    #[cfg(feature = "sqlx-postgres")]
    fn rls_migrations() -> Result<tempfile::TempDir, std::io::Error> {
        let directory = tempfile::tempdir()?;
        std::fs::write(directory.path().join("0001_notes.sql"), RLS_MIGRATION)?;
        Ok(directory)
    }

    #[cfg(feature = "sqlx-postgres")]
    async fn assert_app_role_is_bound_by_rls(
        admin_url: &str,
        app_url: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let admin = sqlx::PgPool::connect(admin_url).await?;
        sqlx::query("INSERT INTO notes VALUES ('alice', 'a'), ('bob', 'b')")
            .execute(&admin)
            .await?;
        let mut app = sqlx::PgConnection::connect(app_url).await?;
        let role =
            sqlx::query("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = current_user")
                .fetch_one(&mut app)
                .await?;
        assert!(!role.try_get::<bool, _>("rolsuper")?);
        assert!(!role.try_get::<bool, _>("rolbypassrls")?);
        sqlx::query("SELECT set_config('app.owner', 'alice', false)")
            .execute(&mut app)
            .await?;
        sqlx::query("INSERT INTO notes VALUES ('alice', 'c')")
            .execute(&mut app)
            .await?;
        let visible: i64 = sqlx::query("SELECT count(*) AS visible FROM notes")
            .fetch_one(&mut app)
            .await?
            .try_get("visible")?;
        assert_eq!(visible, 2);
        let forged = sqlx::query("INSERT INTO notes VALUES ('bob', 'forged')")
            .execute(&mut app)
            .await;
        assert!(forged.is_err(), "RLS allowed a write for another owner");
        app.close().await?;
        admin.close().await;
        Ok(())
    }

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn app_role_is_subject_to_row_level_security() -> Result<(), Box<dyn std::error::Error>> {
        let migrations = rls_migrations()?;
        let fixture = PostgresTestOptions::new()
            .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
            .with_migrations(migrations.path())
            .start()
            .await?;
        let app_url = fixture.app_connection_url().ok_or("missing app URL")?;
        assert!(app_url.starts_with("postgres://app_user:app-secret@"));
        assert_app_role_is_bound_by_rls(fixture.connection_url(), app_url).await
    }

    #[cfg(feature = "sqlx-postgres")]
    async fn database_exists(admin_url: &str, name: &str) -> Result<bool, sqlx::Error> {
        let mut admin = sqlx::PgConnection::connect(admin_url).await?;
        let exists = sqlx::query("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(name)
            .fetch_one(&mut admin)
            .await?
            .try_get::<bool, _>(0)?;
        admin.close().await?;
        Ok(exists)
    }

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn creates_and_drops_a_database_per_test() -> Result<(), Box<dyn std::error::Error>> {
        let migrations = rls_migrations()?;
        let fixture = PostgresTestOptions::new()
            .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
            .with_migrations(migrations.path())
            .start()
            .await?;
        let first = fixture.create_database().await?;
        let second = fixture.create_database().await?;
        assert_ne!(first.name(), second.name());
        assert!(first.name().starts_with("baukit_test_"));
        assert_app_role_is_bound_by_rls(
            first.connection_url(),
            first.app_connection_url().ok_or("missing app URL")?,
        )
        .await?;

        let first_name = first.name().to_owned();
        let second_name = second.name().to_owned();
        let _open = sqlx::PgPool::connect(first.connection_url()).await?;
        first.drop_database().await?;
        drop(second);

        assert!(!database_exists(fixture.connection_url(), &first_name).await?);
        assert!(!database_exists(fixture.connection_url(), &second_name).await?);
        Ok(())
    }

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn external_server_databases_are_dropped() -> Result<(), Box<dyn std::error::Error>> {
        let server = start_postgres().await?;
        let databases = PostgresTestDatabases::new(server.connection_url());
        let database = databases.create().await?;
        let name = database.name().to_owned();
        assert!(database_exists(server.connection_url(), &name).await?);
        assert!(database.app_connection_url().is_none());

        drop(database);

        assert!(!database_exists(server.connection_url(), &name).await?);
        Ok(())
    }

    #[cfg(feature = "sqlx-postgres")]
    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn rejects_an_existing_superuser_as_app_role() -> Result<(), Box<dyn std::error::Error>> {
        let server = start_postgres().await?;
        let result = PostgresTestDatabases::new(server.connection_url())
            .with_app_role(PostgresAppRole::new("postgres", "postgres")?)
            .create()
            .await;
        assert!(matches!(result, Err(PostgresTestError::InvalidAppRole(_))));
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
