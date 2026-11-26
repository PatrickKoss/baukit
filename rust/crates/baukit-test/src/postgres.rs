use std::fmt;

#[cfg(feature = "sqlx-postgres")]
use sqlx::Row as _;
#[cfg(feature = "sqlx-postgres")]
use std::path::{Path, PathBuf};
use testcontainers::{
    ContainerAsync, GenericImage, ImageExt as _,
    core::{IntoContainerPort as _, WaitFor},
};

use crate::StartContainer as _;
use crate::fault::{FaultError, PostgresFault, reserve_host_port};
#[cfg(feature = "sqlx-postgres")]
use crate::postgres_database::{
    PostgresAppRole, PostgresMigrationOwner, PostgresTestDatabase, PostgresTestDatabases,
};
#[cfg(feature = "sqlx-postgres")]
use crate::{CleanupKind, OwnedResourceCheck};

const POSTGRES_IMAGE_NAME: &str = "postgres";
const POSTGRES_IMAGE_TAG: &str =
    "18.7-alpine@sha256:b6972e1115a4ff8eba5d7f7d19184926d1e16d7d1136ae1e1a7ebf286da0f015";
const POSTGRES_PORT: u16 = 5432;
/// Names a running PostgreSQL server URL that [`PostgresTestOptions::start`] uses instead
/// of starting a container.
pub const EXTERNAL_DATABASE_URL_VAR: &str = "BAUKIT_TEST_DATABASE_URL";

/// A running disposable PostgreSQL container and its connection URL.
///
/// Keep this value alive for as long as the database is in use. Dropping it
/// invokes Testcontainers' container cleanup behavior.
pub struct PostgresTestContainer {
    connection_url: String,
    #[cfg(feature = "sqlx-postgres")]
    app_connection_url: Option<String>,
    #[cfg(feature = "sqlx-postgres")]
    owner_connection_url: Option<String>,
    #[cfg(feature = "sqlx-postgres")]
    databases: PostgresTestDatabases,
    container: Option<ContainerAsync<GenericImage>>,
    host_port: Option<u16>,
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

    /// Returns the URL for the migration owner on the `postgres` database.
    ///
    /// `None` unless [`PostgresTestOptions::with_migration_owner`] was set. Connect the
    /// application under test with it to run as the owner, as it does in a CNPG cluster.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn owner_connection_url(&self) -> Option<&str> {
        self.owner_connection_url.as_deref()
    }

    /// Creates a separate database on this container for one test.
    ///
    /// The database gets the container's app role grants and migrations. It is
    /// dropped when the returned value drops.
    ///
    /// # Errors
    ///
    /// Fails like [`PostgresTestDatabases::create`].
    #[cfg(feature = "sqlx-postgres")]
    pub async fn create_database(&self) -> Result<PostgresTestDatabase, PostgresTestError> {
        self.databases.create().await
    }

    /// Returns the underlying Testcontainers guard for advanced test setup.
    ///
    /// `None` when the fixture uses the server named by [`EXTERNAL_DATABASE_URL_VAR`].
    #[must_use]
    pub const fn container(&self) -> Option<&ContainerAsync<GenericImage>> {
        self.container.as_ref()
    }

    /// Returns a handle that stops, restarts, pauses and resumes this container.
    ///
    /// Start the fixture with [`PostgresTestOptions::with_stable_host_port`] when the test
    /// restarts it, so the connection URL stays valid.
    ///
    /// # Errors
    ///
    /// Returns [`FaultError::NoContainer`] when the fixture uses the server named by
    /// [`EXTERNAL_DATABASE_URL_VAR`].
    pub fn fault(&self) -> Result<PostgresFault<'_>, FaultError> {
        let container = self.container.as_ref().ok_or(FaultError::NoContainer)?;
        Ok(PostgresFault::new(container, POSTGRES_PORT, self.host_port))
    }

    /// Splits the fixture into an owned URL and its lifetime guard.
    #[must_use]
    pub fn into_parts(self) -> (String, Option<ContainerAsync<GenericImage>>) {
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
/// The default is the official `postgres:18.7-alpine` image (pinned by digest) with only the
/// `postgres` superuser, which is what [`start_postgres`] starts.
#[derive(Clone, Debug)]
pub struct PostgresTestOptions {
    image_name: String,
    image_tag: String,
    stable_host_port: bool,
    #[cfg(feature = "sqlx-postgres")]
    app_role: Option<PostgresAppRole>,
    #[cfg(feature = "sqlx-postgres")]
    migration_owner: Option<PostgresMigrationOwner>,
    #[cfg(feature = "sqlx-postgres")]
    migrations: Option<PathBuf>,
}

impl Default for PostgresTestOptions {
    fn default() -> Self {
        Self {
            image_name: POSTGRES_IMAGE_NAME.to_owned(),
            image_tag: POSTGRES_IMAGE_TAG.to_owned(),
            stable_host_port: false,
            #[cfg(feature = "sqlx-postgres")]
            app_role: None,
            #[cfg(feature = "sqlx-postgres")]
            migration_owner: None,
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

    /// Replaces the image, for example `postgres` and `18.7-alpine3.23`.
    ///
    /// The image must accept the official image's `POSTGRES_USER`,
    /// `POSTGRES_PASSWORD`, and `POSTGRES_DB` variables.
    #[must_use]
    pub fn with_image(mut self, name: impl Into<String>, tag: impl Into<String>) -> Self {
        self.image_name = name.into();
        self.image_tag = tag.into();
        self
    }

    /// Publishes the server on a host port that survives [`PostgresFault::stop`] and
    /// [`PostgresFault::start`].
    ///
    /// Without it Docker picks a new ephemeral port on every restart.
    #[must_use]
    pub const fn with_stable_host_port(mut self) -> Self {
        self.stable_host_port = true;
        self
    }

    /// Creates a login role without superuser or `BYPASSRLS` before migrations run.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn with_app_role(mut self, role: PostgresAppRole) -> Self {
        self.app_role = Some(role);
        self
    }

    /// Applies SQLx migrations from `path` after the app role exists, as the migration owner
    /// when one is set and as the superuser otherwise.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn with_migrations(mut self, path: impl Into<PathBuf>) -> Self {
        self.migrations = Some(path.into());
        self
    }

    /// Makes `owner` the owner of the `postgres` database and applies migrations as it.
    ///
    /// The owner has the rights CloudNativePG gives an application owner, so a migration that
    /// needs a superuser fails here instead of in the cluster's migrate Job. The superuser
    /// still creates the roles and test databases.
    #[cfg(feature = "sqlx-postgres")]
    #[must_use]
    pub fn with_migration_owner(mut self, owner: PostgresMigrationOwner) -> Self {
        self.migration_owner = Some(owner);
        self
    }

    /// Starts the container and, with `sqlx-postgres`, waits until it accepts
    /// queries, then prepares the app role and migrations.
    ///
    /// When [`EXTERNAL_DATABASE_URL_VAR`] is set, no container starts: the fixture
    /// prepares the server at that URL, which must allow a superuser login. Otherwise
    /// Docker is contacted only when this function is called.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresTestError::Container`] when Docker cannot start the image or
    /// report its address. With `sqlx-postgres`, also `Connect`, `Setup` or `Migrate`
    /// when the server never accepts queries, the app role cannot be prepared, or a
    /// migration fails.
    pub async fn start(self) -> Result<PostgresTestContainer, PostgresTestError> {
        let external = std::env::var(EXTERNAL_DATABASE_URL_VAR)
            .ok()
            .filter(|url| !url.is_empty());
        self.start_on(external).await
    }

    async fn start_on(
        self,
        external_url: Option<String>,
    ) -> Result<PostgresTestContainer, PostgresTestError> {
        let (connection_url, container, host_port) = match external_url {
            Some(url) => (url, None, None),
            None => {
                let (url, container, port) = self.start_container().await?;
                (url, Some(container), Some(port))
            }
        };
        self.prepare(connection_url, container, host_port).await
    }

    async fn start_container(
        &self,
    ) -> Result<(String, ContainerAsync<GenericImage>, u16), PostgresTestError> {
        let request = GenericImage::new(self.image_name.clone(), self.image_tag.clone())
            .with_exposed_port(POSTGRES_PORT.tcp())
            .with_wait_for(WaitFor::message_on_stderr(
                "database system is ready to accept connections",
            ))
            .with_wait_for(WaitFor::message_on_stdout(
                "database system is ready to accept connections",
            ))
            .with_env_var("POSTGRES_USER", "postgres")
            .with_env_var("POSTGRES_PASSWORD", "postgres")
            .with_env_var("POSTGRES_DB", "postgres")
            .with_labels(crate::container_labels());
        let request = if self.stable_host_port {
            request.with_mapped_port(reserve_host_port()?, POSTGRES_PORT.tcp())
        } else {
            request
        };
        let container = request.start_container().await?;
        let host = container.get_host().await?;
        let port = container.get_host_port_ipv4(POSTGRES_PORT).await?;
        Ok((
            format!("postgres://postgres:postgres@{host}:{port}/postgres"),
            container,
            port,
        ))
    }

    #[cfg_attr(
        not(feature = "sqlx-postgres"),
        expect(
            clippy::unused_async,
            reason = "only the sqlx-postgres build awaits setup"
        )
    )]
    async fn prepare(
        self,
        connection_url: String,
        container: Option<ContainerAsync<GenericImage>>,
        host_port: Option<u16>,
    ) -> Result<PostgresTestContainer, PostgresTestError> {
        #[cfg(feature = "sqlx-postgres")]
        {
            let mut databases = PostgresTestDatabases::new(connection_url.clone());
            if let Some(role) = self.app_role {
                databases = databases.with_app_role(role);
            }
            if let Some(owner) = self.migration_owner {
                databases = databases.with_migration_owner(owner);
            }
            if let Some(path) = self.migrations {
                databases = databases.with_migrations(path);
            }
            let urls = databases.prepare_server_database().await?;
            Ok(PostgresTestContainer {
                connection_url,
                app_connection_url: urls.app,
                owner_connection_url: urls.owner,
                databases,
                container,
                host_port,
            })
        }
        #[cfg(not(feature = "sqlx-postgres"))]
        Ok(PostgresTestContainer {
            connection_url,
            container,
            host_port,
        })
    }
}

/// Failure while starting a PostgreSQL fixture or applying its migrations.
#[derive(Debug, thiserror::Error)]
pub enum PostgresTestError {
    /// Testcontainers could not start or inspect the PostgreSQL container.
    #[error("could not start PostgreSQL test container: {0}")]
    Container(#[from] testcontainers::TestcontainersError),
    /// No free host port could be reserved for a stable mapping.
    #[error("could not reserve a host port for PostgreSQL: {0}")]
    HostPort(#[from] std::io::Error),
    /// SQLx could not connect to the newly started PostgreSQL instance.
    #[cfg(feature = "sqlx-postgres")]
    #[error("could not connect to PostgreSQL test container: {0}")]
    Connect(#[source] sqlx::Error),
    /// SQLx could not load or apply the caller's migrations.
    #[cfg(feature = "sqlx-postgres")]
    #[error("could not apply PostgreSQL test migrations: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// An app role or migration owner name or password is not accepted, or an existing role
    /// has more rights than the fixture allows.
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
    /// A database name does not match `[a-z_][a-z0-9_]*` or exceeds the byte limit shown.
    #[cfg(feature = "sqlx-postgres")]
    #[error("invalid PostgreSQL database name: must match [a-z_][a-z0-9_]* and fit {0} bytes")]
    InvalidDatabaseName(usize),
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

/// Starts a disposable PostgreSQL 18.7 container asynchronously.
///
/// The default Testcontainers module credentials and database are all
/// `postgres`. Docker is contacted only when this function is called.
///
/// # Errors
///
/// Fails like [`PostgresTestOptions::start`].
pub async fn start_postgres() -> Result<PostgresTestContainer, PostgresTestError> {
    PostgresTestOptions::new().start().await
}

/// Starts PostgreSQL 18.7 and applies SQLx migrations from `migrations_path`.
///
/// This helper is available with the `sqlx-postgres` feature. Migration files
/// use SQLx's ordinary file naming and checksum rules.
///
/// # Errors
///
/// Fails like [`PostgresTestOptions::start`], including
/// [`PostgresTestError::Migrate`] when a migration cannot be read or applied.
#[cfg(feature = "sqlx-postgres")]
pub async fn start_postgres_with_migrations(
    migrations_path: impl AsRef<Path>,
) -> Result<PostgresTestContainer, PostgresTestError> {
    PostgresTestOptions::new()
        .with_migrations(migrations_path.as_ref())
        .start()
        .await
}

/// Starts PostgreSQL 18.7 and applies SQLx migrations from `migrations_path` as `owner`.
///
/// Shorthand for [`PostgresTestOptions::with_migration_owner`] plus
/// [`PostgresTestOptions::with_migrations`]. Connect the code under test with
/// [`PostgresTestContainer::owner_connection_url`].
///
/// # Errors
///
/// Fails like [`PostgresTestOptions::start`], including [`PostgresTestError::Migrate`] when a
/// migration needs rights the owner does not have.
#[cfg(feature = "sqlx-postgres")]
pub async fn start_postgres_with_owner_migrations(
    migrations_path: impl AsRef<Path>,
    owner: PostgresMigrationOwner,
) -> Result<PostgresTestContainer, PostgresTestError> {
    PostgresTestOptions::new()
        .with_migration_owner(owner)
        .with_migrations(migrations_path.as_ref())
        .start()
        .await
}

/// Lists direct foreign keys to `user_root_table` that violate declared cleanup.
///
/// Registry names may be unqualified table names or schema-qualified names.
/// Unregistered direct references must use `ON DELETE CASCADE`.
///
/// # Errors
///
/// Returns [`sqlx::Error::RowNotFound`] when `user_root_table` does not exist, and
/// any other [`sqlx::Error`] from the catalog queries.
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

    let mut registered = std::collections::BTreeMap::new();
    for resource in resources {
        let table = sqlx::query_scalar::<_, String>(
            "SELECT quote_ident(namespace.nspname) || '.' || quote_ident(table_row.relname) \
             FROM pg_class AS table_row \
             JOIN pg_namespace AS namespace ON namespace.oid = table_row.relnamespace \
             WHERE table_row.oid = to_regclass($1)",
        )
        .bind(resource.name)
        .fetch_optional(pool)
        .await?;
        if let Some(table) = table {
            registered.insert(table, resource.cleanup);
        }
    }

    let rows = sqlx::query(
        "SELECT constraint_row.conname AS constraint_name, \
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
        let qualified_table = row.try_get::<String, _>("qualified_table")?;
        let declared_cleanup = registered
            .get(&qualified_table)
            .copied()
            .unwrap_or(CleanupKind::Cascade);
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
mod tests;
