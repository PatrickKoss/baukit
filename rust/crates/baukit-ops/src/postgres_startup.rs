//! PostgreSQL connects that wait for the server during startup.

use std::{str::FromStr, time::Duration};

use baukit_config::DatabaseConfig;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

use baukit_resilience::startup::{RetryAttempt, StartupError, retry_within};

use crate::postgres_pool::{connect_options, pool_options};

const DEPENDENCY: &str = "postgres";
const MIGRATION_CONNECTIONS: u32 = 1;
const TOO_MANY_CONNECTIONS: &str = "53300";
const CANNOT_CONNECT_NOW: &str = "57P03";
const ADMIN_SHUTDOWN: &str = "57P01";

/// Connects a pool like [`crate::connect_pool`], retrying transient failures
/// within `budget`.
///
/// Pass `BaukitConfig::startup.budget`. Retries connection refusals, DNS and
/// other I/O errors, pool timeouts, and the server codes `57P03`
/// (`cannot_connect_now`), `57P01` (`admin_shutdown`) and `53300`
/// (`too_many_connections`). Bad credentials, a missing database, TLS and URL
/// errors fail at once.
///
/// # Errors
///
/// [`StartupError::Permanent`] for a non-transient error and
/// [`StartupError::BudgetExhausted`] when the server stays unreachable.
pub async fn connect_with_backoff(
    config: &DatabaseConfig,
    budget: Duration,
) -> Result<PgPool, StartupError<sqlx::Error>> {
    let options = connect_options(config).map_err(StartupError::Permanent)?;
    let pool = pool_options(config);
    retry_within(
        DEPENDENCY,
        budget,
        is_transient_connect_error,
        log_retry,
        || pool.clone().connect_with(options.clone()),
    )
    .await
}

/// Connects a single-connection pool for a migration binary, retrying like
/// [`connect_with_backoff`].
///
/// The session keeps the configured `lock_timeout` but not the
/// `statement_timeout` or `idle_in_transaction_session_timeout`, so a long
/// migration step is not cancelled at the request-sized limit.
///
/// # Errors
///
/// The errors of [`connect_with_backoff`].
pub async fn connect_for_migrations(
    config: &DatabaseConfig,
    budget: Duration,
) -> Result<PgPool, StartupError<sqlx::Error>> {
    let options = migration_connect_options(config).map_err(StartupError::Permanent)?;
    let pool = PgPoolOptions::new()
        .max_connections(MIGRATION_CONNECTIONS)
        .min_connections(0)
        .acquire_timeout(config.acquire_timeout);
    retry_within(
        DEPENDENCY,
        budget,
        is_transient_connect_error,
        log_retry,
        || pool.clone().connect_with(options.clone()),
    )
    .await
}

/// Returns `true` when a connect error may clear up by waiting.
#[must_use]
pub fn is_transient_connect_error(error: &sqlx::Error) -> bool {
    match error {
        sqlx::Error::Io(_) | sqlx::Error::PoolTimedOut => true,
        sqlx::Error::Database(database) => database.code().is_some_and(|code| {
            [TOO_MANY_CONNECTIONS, CANNOT_CONNECT_NOW, ADMIN_SHUTDOWN].contains(&code.as_ref())
        }),
        _ => false,
    }
}

fn log_retry(retry: &RetryAttempt<'_, sqlx::Error>) {
    tracing::warn!(
        dependency = retry.dependency,
        attempt = retry.attempt,
        delay_ms = u64::try_from(retry.delay.as_millis()).unwrap_or(u64::MAX),
        error = %retry.error,
        "dependency unavailable at startup, retrying"
    );
}

fn migration_connect_options(config: &DatabaseConfig) -> Result<PgConnectOptions, sqlx::Error> {
    let options = PgConnectOptions::from_str(config.url.expose())?;
    Ok(options.options([(
        "lock_timeout",
        format!("{}ms", config.lock_timeout.as_millis()),
    )]))
}

#[cfg(test)]
#[path = "postgres_startup/tests.rs"]
mod tests;
