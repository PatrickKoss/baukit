use std::time::Duration;

use serde::Deserialize;

use crate::{
    Secret, Validate, ValidationError, ValidationErrors, duration_seconds, require_non_zero,
    validation_result,
};

const DEFAULT_MAX_CONNECTIONS: u32 = 10;
const DEFAULT_MIN_CONNECTIONS: u32 = 1;
const DEFAULT_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(29);
const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const DEFAULT_IDLE_IN_TRANSACTION_SESSION_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_MAX_LIFETIME: Duration = Duration::from_secs(30 * 60);
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Plain database connection, pool, and session-timeout configuration.
///
/// Every duration deserializes from integer seconds.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// Database connection URL. Formatting the wrapper always redacts it.
    pub url: Secret<String>,
    /// Maximum number of connections in the pool.
    pub max_connections: u32,
    /// Minimum number of connections retained by the pool.
    pub min_connections: u32,
    /// Maximum time to wait for a pool connection.
    #[serde(with = "duration_seconds")]
    pub acquire_timeout: Duration,
    /// PostgreSQL `statement_timeout`. Must stay below `http.request_timeout`.
    #[serde(with = "duration_seconds")]
    pub statement_timeout: Duration,
    /// PostgreSQL `lock_timeout`.
    #[serde(with = "duration_seconds")]
    pub lock_timeout: Duration,
    /// PostgreSQL `idle_in_transaction_session_timeout`.
    #[serde(with = "duration_seconds")]
    pub idle_in_transaction_session_timeout: Duration,
    /// Maximum lifetime of a pooled connection before it is closed.
    #[serde(with = "duration_seconds")]
    pub max_lifetime: Duration,
    /// Time an idle pooled connection is kept before it is closed.
    #[serde(with = "duration_seconds")]
    pub idle_timeout: Duration,
}

impl DatabaseConfig {
    pub(crate) fn validate_request_timeout(
        &self,
        request_timeout: Duration,
    ) -> Result<(), ValidationErrors> {
        if self.statement_timeout < request_timeout {
            return Ok(());
        }
        validation_result(vec![ValidationError::new(
            "statement_timeout",
            "must be below http.request_timeout",
        )])
    }
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: Secret::new("postgres://localhost/app".to_owned()),
            max_connections: DEFAULT_MAX_CONNECTIONS,
            min_connections: DEFAULT_MIN_CONNECTIONS,
            acquire_timeout: DEFAULT_ACQUIRE_TIMEOUT,
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT,
            lock_timeout: DEFAULT_LOCK_TIMEOUT,
            idle_in_transaction_session_timeout: DEFAULT_IDLE_IN_TRANSACTION_SESSION_TIMEOUT,
            max_lifetime: DEFAULT_MAX_LIFETIME,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
        }
    }
}

impl Validate for DatabaseConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if self.url.expose().trim().is_empty() {
            errors.push(ValidationError::new("url", "must not be empty"));
        }
        require_non_zero(self.max_connections, "max_connections", &mut errors);
        if self.min_connections > self.max_connections {
            errors.push(ValidationError::new(
                "min_connections",
                "must not exceed max_connections",
            ));
        }
        for (value, path) in [
            (self.acquire_timeout, "acquire_timeout"),
            (self.statement_timeout, "statement_timeout"),
            (self.lock_timeout, "lock_timeout"),
            (
                self.idle_in_transaction_session_timeout,
                "idle_in_transaction_session_timeout",
            ),
            (self.max_lifetime, "max_lifetime"),
            (self.idle_timeout, "idle_timeout"),
        ] {
            require_non_zero(value.as_secs(), path, &mut errors);
        }
        validation_result(errors)
    }
}
