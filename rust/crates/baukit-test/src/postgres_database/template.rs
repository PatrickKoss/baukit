use crate::postgres::PostgresTestError;

use super::{
    MAX_IDENTIFIER_BYTES, PostgresTestDatabase, PostgresTestDatabases, drop_database,
    is_plain_identifier,
};

/// Name of a database on the test server, limited to `[a-z_][a-z0-9_]*` and 63 bytes.
///
/// A value of this type needs no quoting, so the template helpers can put it into a statement
/// without splicing text the caller controls.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresDatabaseName(String);

impl PostgresDatabaseName {
    /// Validates a database name.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresTestError::InvalidDatabaseName`] when the name is empty, longer than
    /// 63 bytes, or not `[a-z_][a-z0-9_]*`.
    pub fn new(name: impl Into<String>) -> Result<Self, PostgresTestError> {
        let name = name.into();
        if is_plain_identifier(&name) {
            return Ok(Self(name));
        }
        Err(PostgresTestError::InvalidDatabaseName(MAX_IDENTIFIER_BYTES))
    }

    /// Returns the name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PostgresTestDatabase {
    /// Returns this database's name as a template name.
    #[must_use]
    pub fn as_template(&self) -> PostgresDatabaseName {
        PostgresDatabaseName(self.name.clone())
    }
}

impl PostgresTestDatabases {
    /// Creates a uniquely named database with `CREATE DATABASE ... TEMPLATE`.
    ///
    /// The clone keeps the template's schema, data and grants, so it needs no migrations. The
    /// template must have no open connections while PostgreSQL copies it, so close its pools
    /// first. The clone is dropped when the returned value drops.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresTestError::InvalidConnectionUrl`] when the admin URL cannot name the
    /// new database, `Connect` when the server is unreachable, and `Setup` when the template is
    /// missing, still in use, or the statement fails.
    pub async fn create_from_template(
        &self,
        template: &PostgresDatabaseName,
    ) -> Result<PostgresTestDatabase, PostgresTestError> {
        self.create_with(|name| {
            format!(
                "CREATE DATABASE \"{name}\" TEMPLATE \"{}\"",
                template.as_str()
            )
        })
        .await
    }

    /// Drops `name` with `DROP DATABASE IF EXISTS ... WITH (FORCE)`.
    ///
    /// # Errors
    ///
    /// Returns [`PostgresTestError::Connect`] when the admin connection fails and
    /// [`PostgresTestError::Setup`] when the statement fails.
    pub async fn drop_named(&self, name: &PostgresDatabaseName) -> Result<(), PostgresTestError> {
        drop_database(&self.admin_url, name.as_str()).await
    }
}

#[cfg(test)]
mod tests {
    use sqlx::{Connection as _, PgConnection, Row as _};

    use super::*;
    use crate::start_postgres;

    #[test]
    fn accepts_plain_names_and_rejects_anything_that_needs_quoting() {
        assert!(PostgresDatabaseName::new("template_1").is_ok());
        for name in [
            "",
            "Template",
            "1db",
            "a-b",
            "a\"b",
            "a; DROP DATABASE x",
            "a b",
            &"a".repeat(64),
        ] {
            assert!(
                matches!(
                    PostgresDatabaseName::new(name),
                    Err(PostgresTestError::InvalidDatabaseName(_))
                ),
                "accepted {name:?}"
            );
        }
    }

    async fn scalar(url: &str, sql: &'static str) -> Result<i64, sqlx::Error> {
        let mut connection = PgConnection::connect(url).await?;
        let value = sqlx::query(sql)
            .fetch_one(&mut connection)
            .await?
            .try_get(0)?;
        connection.close().await?;
        Ok(value)
    }

    async fn run(url: &str, sql: &'static str) -> Result<(), sqlx::Error> {
        let mut connection = PgConnection::connect(url).await?;
        sqlx::query(sql).execute(&mut connection).await?;
        connection.close().await
    }

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn clones_schema_and_data_and_isolates_the_copies()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = start_postgres().await?;
        let databases = PostgresTestDatabases::new(server.connection_url());
        let template = databases.create().await?;
        run(
            template.connection_url(),
            "CREATE TABLE seeded (id BIGINT PRIMARY KEY)",
        )
        .await?;
        run(template.connection_url(), "INSERT INTO seeded VALUES (1)").await?;

        let first = databases
            .create_from_template(&template.as_template())
            .await?;
        let second = databases
            .create_from_template(&template.as_template())
            .await?;
        assert_ne!(first.name(), second.name());

        run(first.connection_url(), "INSERT INTO seeded VALUES (2)").await?;
        let count = "SELECT count(*) FROM seeded";
        assert_eq!(scalar(first.connection_url(), count).await?, 2);
        assert_eq!(scalar(second.connection_url(), count).await?, 1);
        assert_eq!(scalar(template.connection_url(), count).await?, 1);

        let first_name = PostgresDatabaseName::new(first.name())?;
        databases.drop_named(&first_name).await?;
        let exists = "SELECT count(*) FROM pg_database WHERE datname LIKE 'baukit_test_%'";
        assert_eq!(scalar(server.connection_url(), exists).await?, 2);
        drop(first);
        drop(second);
        assert_eq!(scalar(server.connection_url(), exists).await?, 1);
        Ok(())
    }

    #[tokio::test]
    #[ignore = "requires a reachable Docker daemon and may pull the PostgreSQL image"]
    async fn a_missing_template_is_a_setup_error() -> Result<(), Box<dyn std::error::Error>> {
        let server = start_postgres().await?;
        let result = PostgresTestDatabases::new(server.connection_url())
            .create_from_template(&PostgresDatabaseName::new("no_such_template")?)
            .await;
        assert!(matches!(result, Err(PostgresTestError::Setup(_))));
        Ok(())
    }
}
