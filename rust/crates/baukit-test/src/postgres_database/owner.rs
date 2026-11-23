use std::fmt;

use sqlx::{PgConnection, Row as _};

use crate::postgres::PostgresTestError;

use super::{execute, validate_credentials};

const DEFAULT_OWNER: &str = "app";

/// Login role that owns the test database and applies migrations.
///
/// It gets the rights CloudNativePG gives an application owner: `LOGIN`, ownership of the
/// database (and so of its `public` schema), and nothing else. It is not a superuser and has no
/// `CREATEDB`, `REPLICATION` or `BYPASSRLS`, so a migration that only works as a superuser fails
/// in the test as it would in the cluster. [`PostgresMigrationOwner::with_create_role`] adds
/// `CREATEROLE` for products that declare `[postgres] owner_create_role = true`.
///
/// The default is `app` with password `app`, the owner name of a CNPG `bootstrap.initdb`.
#[derive(Clone, Eq, PartialEq)]
pub struct PostgresMigrationOwner {
    name: String,
    password: String,
    create_role: bool,
}

impl Default for PostgresMigrationOwner {
    fn default() -> Self {
        Self {
            name: DEFAULT_OWNER.to_owned(),
            password: DEFAULT_OWNER.to_owned(),
            create_role: false,
        }
    }
}

impl PostgresMigrationOwner {
    /// Validates a role name and password with the rules of [`super::PostgresAppRole::new`].
    ///
    /// # Errors
    ///
    /// Returns [`PostgresTestError::InvalidAppRole`] when the name or password breaks them.
    pub fn new(
        name: impl Into<String>,
        password: impl Into<String>,
    ) -> Result<Self, PostgresTestError> {
        let name = name.into();
        let password = password.into();
        validate_credentials(&name, &password)?;
        Ok(Self {
            name,
            password,
            create_role: false,
        })
    }

    /// Gives the owner `CREATEROLE`, as `[postgres] owner_create_role = true` does in a
    /// dedicated CNPG cluster.
    ///
    /// Since PostgreSQL 16 the owner then holds only `ADMIN` on the roles it creates. A
    /// migration that switches into such a role or hands it objects must first run
    /// `GRANT <role> TO CURRENT_USER` or `SET createrole_self_grant = 'set, inherit'`.
    #[must_use]
    pub const fn with_create_role(mut self) -> Self {
        self.create_role = true;
        self
    }

    /// Returns the role name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn password(&self) -> &str {
        &self.password
    }

    /// Creates the role if it is missing and rejects an existing role with other attributes.
    pub(super) async fn ensure(&self, admin: &mut PgConnection) -> Result<(), PostgresTestError> {
        let create_role = if self.create_role {
            "CREATEROLE"
        } else {
            "NOCREATEROLE"
        };
        execute(
            admin,
            format!(
                "DO $$ BEGIN \
                     CREATE ROLE \"{name}\" LOGIN PASSWORD '{password}' \
                         NOSUPERUSER NOCREATEDB NOREPLICATION NOBYPASSRLS {create_role}; \
                 EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; \
                 END $$",
                name = self.name,
                password = self.password,
            ),
        )
        .await?;
        let row = sqlx::query(
            "SELECT rolsuper OR rolcreatedb OR rolreplication OR rolbypassrls AS elevated, \
                    rolcreaterole \
             FROM pg_roles WHERE rolname = $1",
        )
        .bind(&self.name)
        .fetch_one(&mut *admin)
        .await
        .map_err(PostgresTestError::Setup)?;
        let elevated: bool = row.try_get("elevated").map_err(PostgresTestError::Setup)?;
        let create_role: bool = row
            .try_get("rolcreaterole")
            .map_err(PostgresTestError::Setup)?;
        if elevated || create_role != self.create_role {
            return Err(PostgresTestError::InvalidAppRole(
                "existing migration owner has attributes a CNPG owner does not",
            ));
        }
        Ok(())
    }

    /// Makes the role the owner of `database`.
    pub(super) async fn own(
        &self,
        admin: &mut PgConnection,
        database: &str,
    ) -> Result<(), PostgresTestError> {
        execute(
            admin,
            format!(
                "ALTER DATABASE \"{database}\" OWNER TO \"{name}\"",
                name = self.name
            ),
        )
        .await
    }
}

impl fmt::Debug for PostgresMigrationOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PostgresMigrationOwner")
            .field("name", &self.name)
            .field("password", &"<redacted>")
            .field("create_role", &self.create_role)
            .finish()
    }
}
