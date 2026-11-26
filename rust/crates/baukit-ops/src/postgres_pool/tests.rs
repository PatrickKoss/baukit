use std::time::Duration;

use super::*;
use baukit_config::{DatabaseConfig, Secret};
use baukit_test::start_postgres;

const SHORT_STATEMENT_TIMEOUT: Duration = Duration::from_secs(1);
const QUERY_CANCELED: &str = "57014";

fn config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: Secret::new(url.to_owned()),
        max_connections: 1,
        min_connections: 0,
        statement_timeout: SHORT_STATEMENT_TIMEOUT,
        lock_timeout: Duration::from_secs(2),
        idle_in_transaction_session_timeout: Duration::from_secs(3),
        ..DatabaseConfig::default()
    }
}

#[test]
fn connect_options_carry_session_timeouts() {
    let options = connect_options(&config("postgres://localhost/app")).expect("valid URL");
    let rendered = options.get_options().expect("startup options set");
    assert!(rendered.contains("statement_timeout=1000ms"), "{rendered}");
    assert!(rendered.contains("lock_timeout=2000ms"), "{rendered}");
    assert!(
        rendered.contains("idle_in_transaction_session_timeout=3000ms"),
        "{rendered}"
    );
}

#[test]
fn pool_options_carry_limits_and_lifetimes() {
    let database = DatabaseConfig::default();
    let options = pool_options(&database);
    assert_eq!(options.get_max_connections(), database.max_connections);
    assert_eq!(options.get_min_connections(), database.min_connections);
    assert_eq!(options.get_acquire_timeout(), database.acquire_timeout);
    assert_eq!(options.get_max_lifetime(), Some(database.max_lifetime));
    assert_eq!(options.get_idle_timeout(), Some(database.idle_timeout));
}

#[test]
fn invalid_url_is_rejected() {
    let error = connect_options(&config("not a url")).expect_err("invalid URL");
    assert!(matches!(error, sqlx::Error::Configuration(_)), "{error}");
}

#[tokio::test]
async fn lazy_pool_does_not_connect() {
    let pool = connect_pool_lazy(&config("postgres://localhost/unused")).expect("valid URL");
    assert_eq!(pool.size(), 0);
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn statement_timeout_cancels_long_query_and_returns_connection() {
    let container = start_postgres().await.expect("start PostgreSQL");
    let pool = connect_pool(&config(container.connection_url()))
        .await
        .expect("connect pool");

    let mut connection = crate::acquire(&pool).await.expect("acquire");
    let error = sqlx::query("SELECT pg_sleep(60)")
        .execute(&mut *connection)
        .await
        .expect_err("statement timeout");
    let code = error
        .as_database_error()
        .and_then(|database| database.code().map(|code| code.into_owned()));
    assert_eq!(code.as_deref(), Some(QUERY_CANCELED), "{error}");
    drop(connection);

    let mut reused = crate::acquire(&pool)
        .await
        .expect("connection back in pool");
    let value: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&mut *reused)
        .await
        .expect("query after timeout");
    assert_eq!(value, 1);
    assert_eq!(pool.size(), 1);
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn session_reports_configured_timeouts() {
    let container = start_postgres().await.expect("start PostgreSQL");
    let pool = connect_pool(&config(container.connection_url()))
        .await
        .expect("connect pool");
    let mut connection = crate::acquire(&pool).await.expect("acquire");

    for (query, expected) in [
        ("SHOW statement_timeout", "1s"),
        ("SHOW lock_timeout", "2s"),
        ("SHOW idle_in_transaction_session_timeout", "3s"),
    ] {
        let value: String = sqlx::query_scalar(query)
            .fetch_one(&mut *connection)
            .await
            .expect("show setting");
        assert_eq!(value, expected, "{query}");
    }
}
