use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use reqwest::Url;
use sqlx::{AssertSqlSafe, Connection as _, PgConnection, Row as _};

use crate::postgres::PostgresTestError;

const MAX_IDENTIFIER_BYTES: usize = 63;
const DATABASE_NAME_PREFIX: &str = "baukit_test_";
const READINESS_TIMEOUT: Duration = Duration::from_secs(60);
const READINESS_POLL_INTERVAL: Duration = Duration::from_millis(200);
const DROP_TIMEOUT: Duration = Duration::from_secs(30);

/// Login role the application connects as in tests.
///
/// The role is created without superuser and without `BYPASSRLS`, so
/// row-level security policies apply to it. The password appears in
/// connection URLs, so it is limited to URL-unreserved characters.
#[derive(Clone, Eq, PartialEq)]
pub struct PostgresAppRole {
    name: String,
    password: String,
}

impl PostgresAppRole {
    /// Validates a role name and password.
    ///
    /// The name must match `[a-z_][a-z0-9_]*` and fit PostgreSQL's 63-byte
    /// identifier limit. The password must be non-empty and use only
    /// `A-Z`, `a-z`, `0-9`, `.`, `_`, `~`, and `-`.
    pub fn new(
        name: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, PostgresTestError> {
        let name = name.into();
        let password = password.into();
        if !is_valid_role_name(&name) {
            return Err(PostgresTestError::InvalidAppRole(
                "name must match [a-z_][a-z0-9_]* and fit 63 bytes",
            ));
        }
        if !is_valid_password(&password) {
            return Err(PostgresTestError::InvalidAppRole(
                "password must be non-empty URL-unreserved characters",
            ));
        }
        Ok(Self { name, password })
    }

    /// Returns the role name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl fmt::Debug for PostgresAppRole {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresAppRole")
            .field("name", &self.name)
            .field("password", &"<redacted>")
            .finish()
    }
}

fn is_valid_role_name(name: &str) -> bool {
    let mut characters = name.chars();
    let Some(first) = characters.next() else {
        return false;
    };
    name.len() <= MAX_IDENTIFIER_BYTES
        && (first.is_ascii_lowercase() || first == '_')
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

fn is_valid_password(password: &str) -> bool {
    !password.is_empty()
        && password
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "._~-".contains(character))
}

/// Creates one database per test on a PostgreSQL server the test does not own.
///
/// `admin_url` must connect as a role allowed to create databases and roles,
/// for example a `DATABASE_URL` pointing at a shared CI server. Every database
/// this value creates is dropped when its [`PostgresTestDatabase`] drops.
#[derive(Clone)]
pub struct PostgresTestDatabases {
    admin_url: String,
    app_role: Option<PostgresAppRole>,
    migrations: Option<PathBuf>,
}

impl PostgresTestDatabases {
    /// Uses `admin_url` to create and drop test databases.
    #[must_use]
    pub fn new(admin_url: impl Into<String>) -> Self {
        Self {
            admin_url: admin_url.into(),
            app_role: None,
            migrations: None,
        }
    }

    /// Creates `role` if it is missing and grants it table and sequence access
    /// in every new database.
    #[must_use]
    pub fn with_app_role(mut self, role: PostgresAppRole) -> Self {
        self.app_role = Some(role);
        self
    }

    /// Applies SQLx migrations from `path` to every new database as the admin role.
    #[must_use]
    pub fn with_migrations(mut self, path: impl Into<PathBuf>) -> Self {
        self.migrations = Some(path.into());
        self
    }

    /// Creates a uniquely named database, grants the app role, and runs migrations.
    pub async fn create(&self) -> Result<PostgresTestDatabase, PostgresTestError> {
        let name = format!("{DATABASE_NAME_PREFIX}{}", uuid::Uuid::new_v4().simple());
        let connection_url = with_database(&self.admin_url, &name)?;
        let app_connection_url = self.app_url(&name)?;
        let mut admin = connect_when_ready(&self.admin_url).await?;
        self.ensure_app_role(&mut admin).await?;
        execute(&mut admin, format!("CREATE DATABASE \"{name}\"")).await?;
        close(admin).await;
        let database = PostgresTestDatabase {
            name,
            connection_url,
            app_connection_url,
            admin_url: self.admin_url.clone(),
            dropped: false,
        };
        self.prepare(&database.connection_url).await?;
        Ok(database)
    }

    pub(crate) async fn prepare_server_database(
        &self,
    ) -> Result<Option<String>, PostgresTestError> {
        let mut admin = connect_when_ready(&self.admin_url).await?;
        self.ensure_app_role(&mut admin).await?;
        close(admin).await;
        self.prepare(&self.admin_url).await?;
        let database = Url::parse(&self.admin_url)
            .map_err(|_| PostgresTestError::InvalidConnectionUrl)?
            .path()
            .trim_start_matches('/')
            .to_owned();
        self.app_url(&database)
    }

    async fn prepare(&self, database_url: &str) -> Result<(), PostgresTestError> {
        let mut connection = PgConnection::connect(database_url)
            .await
            .map_err(PostgresTestError::Connect)?;
        if let Some(role) = &self.app_role {
            grant_app_role(&mut connection, role).await?;
        }
        if let Some(path) = &self.migrations {
            sqlx::migrate::Migrator::new(path.as_path())
                .await?
                .run(&mut connection)
                .await?;
        }
        close(connection).await;
        Ok(())
    }

    async fn ensure_app_role(&self, admin: &mut PgConnection) -> Result<(), PostgresTestError> {
        let Some(role) = &self.app_role else {
            return Ok(());
        };
        execute(
            admin,
            format!(
                "DO $$ BEGIN \
                     CREATE ROLE \"{name}\" LOGIN PASSWORD '{password}' NOSUPERUSER NOBYPASSRLS; \
                 EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
                 END $$",
                name = role.name,
                password = role.password,
            ),
        )
        .await?;
        let row = sqlx::query("SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = $1")
            .bind(&role.name)
            .fetch_one(&mut *admin)
            .await
            .map_err(PostgresTestError::Setup)?;
        let is_superuser: bool = row.try_get("rolsuper").map_err(PostgresTestError::Setup)?;
        let bypasses_rls: bool = row
            .try_get("rolbypassrls")
            .map_err(PostgresTestError::Setup)?;
        if is_superuser || bypasses_rls {
            return Err(PostgresTestError::InvalidAppRole(
                "existing role is a superuser or bypasses row-level security",
            ));
        }
        Ok(())
    }

    fn app_url(&self, database: &str) -> Result<Option<String>, PostgresTestError> {
        let Some(role) = &self.app_role else {
            return Ok(None);
        };
        let mut url = Url::parse(&with_database(&self.admin_url, database)?)
            .map_err(|_| PostgresTestError::InvalidConnectionUrl)?;
        url.set_username(&role.name)
            .and_then(|()| url.set_password(Some(&role.password)))
            .map_err(|()| PostgresTestError::InvalidConnectionUrl)?;
        Ok(Some(url.into()))
    }
}

impl fmt::Debug for PostgresTestDatabases {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresTestDatabases")
            .field("app_role", &self.app_role)
            .field("migrations", &self.migrations)
            .finish_non_exhaustive()
    }
}

/// A database created for one test. Dropping it drops the database.
///
/// Call [`PostgresTestDatabase::drop_database`] to drop it inside the async
/// test and see errors. The `Drop` fallback runs the same statement on a
/// separate thread and ignores failures.
pub struct PostgresTestDatabase {
    name: String,
    connection_url: String,
    app_connection_url: Option<String>,
    admin_url: String,
    dropped: bool,
}

impl PostgresTestDatabase {
    /// Returns the generated database name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the admin URL for this database.
    #[must_use]
    pub fn connection_url(&self) -> &str {
        &self.connection_url
    }

    /// Returns the app role URL for this database, when an app role is set.
    #[must_use]
    pub fn app_connection_url(&self) -> Option<&str> {
        self.app_connection_url.as_deref()
    }

    /// Drops the database, closing any connections still open to it.
    pub async fn drop_database(mut self) -> Result<(), PostgresTestError> {
        self.dropped = true;
        drop_database(&self.admin_url, &self.name).await
    }
}

impl fmt::Debug for PostgresTestDatabase {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresTestDatabase")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

impl Drop for PostgresTestDatabase {
    fn drop(&mut self) {
        if self.dropped {
            return;
        }
        let admin_url = self.admin_url.clone();
        let name = self.name.clone();
        let cleanup = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            let _ = runtime.block_on(async {
                tokio::time::timeout(DROP_TIMEOUT, drop_database(&admin_url, &name)).await
            });
        });
        let _ = cleanup.join();
    }
}

async fn drop_database(admin_url: &str, name: &str) -> Result<(), PostgresTestError> {
    let mut admin = PgConnection::connect(admin_url)
        .await
        .map_err(PostgresTestError::Connect)?;
    execute(
        &mut admin,
        format!("DROP DATABASE IF EXISTS \"{name}\" WITH (FORCE)"),
    )
    .await?;
    close(admin).await;
    Ok(())
}

async fn grant_app_role(
    connection: &mut PgConnection,
    role: &PostgresAppRole,
) -> Result<(), PostgresTestError> {
    let name = &role.name;
    for statement in [
        format!("GRANT USAGE ON SCHEMA public TO \"{name}\""),
        format!(
            "ALTER DEFAULT PRIVILEGES IN SCHEMA public \
             GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO \"{name}\""
        ),
        format!(
            "ALTER DEFAULT PRIVILEGES IN SCHEMA public \
             GRANT USAGE, SELECT ON SEQUENCES TO \"{name}\""
        ),
    ] {
        execute(connection, statement).await?;
    }
    Ok(())
}

async fn connect_when_ready(url: &str) -> Result<PgConnection, PostgresTestError> {
    let deadline = Instant::now() + READINESS_TIMEOUT;
    loop {
        match PgConnection::connect(url).await {
            Ok(connection) => return Ok(connection),
            Err(error) if Instant::now() >= deadline => {
                return Err(PostgresTestError::Connect(error));
            }
            Err(_) => tokio::time::sleep(READINESS_POLL_INTERVAL).await,
        }
    }
}

async fn execute(
    connection: &mut PgConnection,
    statement: String,
) -> Result<(), PostgresTestError> {
    sqlx::query(AssertSqlSafe(statement))
        .execute(connection)
        .await
        .map_err(PostgresTestError::Setup)?;
    Ok(())
}

async fn close(connection: PgConnection) {
    let _ = connection.close().await;
}

fn with_database(url: &str, database: &str) -> Result<String, PostgresTestError> {
    let mut url = Url::parse(url).map_err(|_| PostgresTestError::InvalidConnectionUrl)?;
    url.set_path(&format!("/{database}"));
    Ok(url.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_plain_role_names_and_url_safe_passwords() {
        let role = PostgresAppRole::new("app_user", "secret-1.2_~").expect("valid role");
        assert_eq!(role.name(), "app_user");
    }

    #[test]
    fn rejects_role_names_that_need_quoting() {
        for name in ["", "App", "1app", "app-user", "app\"user", &"a".repeat(64)] {
            assert!(
                matches!(
                    PostgresAppRole::new(name, "secret"),
                    Err(PostgresTestError::InvalidAppRole(_))
                ),
                "accepted {name:?}"
            );
        }
    }

    #[test]
    fn rejects_passwords_outside_the_url_unreserved_set() {
        for password in ["", "pass word", "p'w", "p@w", "p:w"] {
            assert!(
                matches!(
                    PostgresAppRole::new("app", password),
                    Err(PostgresTestError::InvalidAppRole(_))
                ),
                "accepted {password:?}"
            );
        }
    }

    #[test]
    fn debug_output_hides_passwords_and_urls() {
        let role = PostgresAppRole::new("app", "hidden-secret").expect("valid role");
        let databases = PostgresTestDatabases::new("postgres://admin:admin-secret@db/postgres")
            .with_app_role(role.clone());
        for rendered in [format!("{role:?}"), format!("{databases:?}")] {
            assert!(!rendered.contains("hidden-secret"), "{rendered}");
            assert!(!rendered.contains("admin-secret"), "{rendered}");
        }
    }

    #[test]
    fn app_url_swaps_credentials_and_database() {
        let databases = PostgresTestDatabases::new("postgres://admin:pw@db:5432/postgres")
            .with_app_role(PostgresAppRole::new("app", "secret").expect("valid role"));
        assert_eq!(
            databases.app_url("baukit_test_1").expect("url"),
            Some("postgres://app:secret@db:5432/baukit_test_1".to_owned())
        );
    }

    #[test]
    fn rejects_unparseable_admin_urls() {
        assert!(matches!(
            with_database("not a url", "db"),
            Err(PostgresTestError::InvalidConnectionUrl)
        ));
    }
}
