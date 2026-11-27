//! PostgreSQL pool construction from [`DatabaseConfig`].

use std::{str::FromStr, time::Duration};

use baukit_config::DatabaseConfig;
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};

/// Parses the configured URL and adds the session timeouts as startup parameters.
///
/// Every connection opened with these options runs with the configured
/// `statement_timeout`, `lock_timeout` and `idle_in_transaction_session_timeout`.
///
/// # Errors
///
/// Returns [`sqlx::Error::Configuration`] when the URL is not a valid PostgreSQL URL.
pub fn connect_options(config: &DatabaseConfig) -> Result<PgConnectOptions, sqlx::Error> {
    let options = PgConnectOptions::from_str(config.url.expose())?;
    Ok(options.options([
        ("statement_timeout", milliseconds(config.statement_timeout)),
        ("lock_timeout", milliseconds(config.lock_timeout)),
        (
            "idle_in_transaction_session_timeout",
            milliseconds(config.idle_in_transaction_session_timeout),
        ),
    ]))
}

/// Pool options with the configured size limits, acquire timeout and lifetimes.
pub fn pool_options(config: &DatabaseConfig) -> PgPoolOptions {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .min_connections(config.min_connections)
        .acquire_timeout(config.acquire_timeout)
        .max_lifetime(config.max_lifetime)
        .idle_timeout(config.idle_timeout)
}

/// Connects a pool built from [`pool_options`] and [`connect_options`].
///
/// Query through [`crate::acquire`] and [`crate::begin`] so the pool
/// acquisition metrics record every checkout.
///
/// # Errors
///
/// Returns the [`sqlx::Error`] from URL parsing or the initial connection.
pub async fn connect_pool(config: &DatabaseConfig) -> Result<PgPool, sqlx::Error> {
    pool_options(config)
        .connect_with(connect_options(config)?)
        .await
}

/// Like [`connect_pool`], but opens connections on first use.
///
/// # Errors
///
/// Returns [`sqlx::Error::Configuration`] when the URL is invalid.
pub fn connect_pool_lazy(config: &DatabaseConfig) -> Result<PgPool, sqlx::Error> {
    Ok(pool_options(config).connect_lazy_with(connect_options(config)?))
}

fn milliseconds(duration: Duration) -> String {
    format!("{}ms", duration.as_millis())
}

#[cfg(test)]
mod tests;
