use std::{borrow::Cow, error::Error as StdError, fmt, io, time::Duration};

use baukit_config::{DatabaseConfig, Secret};
use baukit_test::start_postgres;
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::Instant,
};

use super::*;

const LATE_START: Duration = Duration::from_secs(5);
const TEST_BUDGET: Duration = Duration::from_secs(60);
const SHORT_BUDGET: Duration = Duration::from_secs(2);
const INVALID_PASSWORD: &str = "28P01";

#[derive(Debug)]
struct FakeDatabaseError(&'static str);

impl fmt::Display for FakeDatabaseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "database error {}", self.0)
    }
}

impl StdError for FakeDatabaseError {}

impl sqlx::error::DatabaseError for FakeDatabaseError {
    fn message(&self) -> &str {
        self.0
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.0))
    }

    fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> sqlx::error::ErrorKind {
        sqlx::error::ErrorKind::Other
    }
}

fn database_error(code: &'static str) -> sqlx::Error {
    sqlx::Error::Database(Box::new(FakeDatabaseError(code)))
}

fn config(url: &str) -> DatabaseConfig {
    DatabaseConfig {
        url: Secret::new(url.to_owned()),
        max_connections: 2,
        min_connections: 1,
        acquire_timeout: Duration::from_secs(1),
        ..DatabaseConfig::default()
    }
}

#[test]
fn transient_errors_are_retried() {
    for error in [
        sqlx::Error::Io(io::Error::from(io::ErrorKind::ConnectionRefused)),
        sqlx::Error::Io(io::Error::other("failed to lookup address information")),
        sqlx::Error::Io(io::Error::from(io::ErrorKind::TimedOut)),
        sqlx::Error::PoolTimedOut,
        database_error("57P03"),
        database_error("57P01"),
        database_error("53300"),
    ] {
        assert!(is_transient_connect_error(&error), "{error}");
    }
}

#[test]
fn auth_and_configuration_errors_fail_fast() {
    for error in [
        database_error(INVALID_PASSWORD),
        database_error("28000"),
        database_error("3D000"),
        sqlx::Error::Configuration("bad url".into()),
        sqlx::Error::Tls("bad certificate".into()),
        sqlx::Error::PoolClosed,
    ] {
        assert!(!is_transient_connect_error(&error), "{error}");
    }
}

#[tokio::test]
async fn invalid_url_fails_without_retrying() {
    let started = Instant::now();
    let error = connect_with_backoff(&config("not a url"), TEST_BUDGET)
        .await
        .expect_err("invalid URL");
    assert!(matches!(
        error,
        StartupError::Permanent(sqlx::Error::Configuration(_))
    ));
    assert!(started.elapsed() < SHORT_BUDGET);

    let error = connect_for_migrations(&config("not a url"), TEST_BUDGET)
        .await
        .expect_err("invalid URL");
    assert!(matches!(
        error,
        StartupError::Permanent(sqlx::Error::Configuration(_))
    ));
}

#[tokio::test]
async fn unreachable_server_exhausts_the_budget() {
    let port = free_port().await;
    let error = connect_with_backoff(&config(&local_url(port, "postgres")), SHORT_BUDGET)
        .await
        .expect_err("nothing listens");
    assert!(error.is_budget_exhausted(), "{error}");
}

#[test]
fn migration_options_drop_request_sized_timeouts() {
    let options = migration_connect_options(&config("postgres://localhost/app")).expect("valid");
    let rendered = options.get_options().expect("startup options set");
    assert!(rendered.contains("lock_timeout=5000ms"), "{rendered}");
    assert!(!rendered.contains("statement_timeout"), "{rendered}");
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn pool_connects_when_postgres_starts_after_the_connect_began() {
    let port = free_port().await;
    let started = Instant::now();
    let url = local_url(port, "postgres");
    let connect =
        tokio::spawn(async move { connect_with_backoff(&config(&url), TEST_BUDGET).await });

    let container = start_postgres().await.expect("start PostgreSQL");
    tokio::time::sleep(LATE_START.saturating_sub(started.elapsed())).await;
    let _proxy = forward(port, container.connection_url()).await;

    let pool = connect
        .await
        .expect("join")
        .expect("connect after late start");
    let value: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("query");
    assert_eq!(value, 1);
    assert!(started.elapsed() >= LATE_START);
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn migration_pool_connects_when_postgres_starts_late() {
    let port = free_port().await;
    let url = local_url(port, "postgres");
    let connect =
        tokio::spawn(async move { connect_for_migrations(&config(&url), TEST_BUDGET).await });

    let container = start_postgres().await.expect("start PostgreSQL");
    tokio::time::sleep(LATE_START).await;
    let _proxy = forward(port, container.connection_url()).await;

    let pool = connect
        .await
        .expect("join")
        .expect("connect after late start");
    let timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .expect("show");
    assert_eq!(timeout, "0");
    assert_eq!(pool.options().get_max_connections(), 1);
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn wrong_password_fails_without_waiting_for_the_budget() {
    let container = start_postgres().await.expect("start PostgreSQL");
    let url = with_password(container.connection_url(), "wrong");
    let started = Instant::now();
    let error = connect_with_backoff(&config(&url), TEST_BUDGET)
        .await
        .expect_err("bad password");
    let StartupError::Permanent(sqlx::Error::Database(database)) = &error else {
        panic!("expected a permanent database error, got {error}");
    };
    assert_eq!(database.code().as_deref(), Some(INVALID_PASSWORD));
    assert!(started.elapsed() < SHORT_BUDGET);
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    listener.local_addr().expect("address").port()
}

fn local_url(port: u16, database: &str) -> String {
    format!("postgres://postgres:postgres@127.0.0.1:{port}/{database}")
}

fn with_password(url: &str, password: &str) -> String {
    let (scheme, rest) = url.split_once("://").expect("scheme");
    let (userinfo, host) = rest.split_once('@').expect("userinfo");
    let user = userinfo.split(':').next().expect("user");
    format!("{scheme}://{user}:{password}@{host}")
}

fn target_address(url: &str) -> String {
    let after_at = url.rsplit_once('@').map_or(url, |(_, host)| host);
    after_at
        .split('/')
        .next()
        .expect("host and port")
        .to_owned()
}

struct Proxy(JoinHandle<()>);

impl Drop for Proxy {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn forward(port: u16, url: &str) -> Proxy {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .await
        .expect("bind proxy");
    let target = target_address(url);
    Proxy(tokio::spawn(async move {
        while let Ok((mut inbound, _)) = listener.accept().await {
            let target = target.clone();
            tokio::spawn(async move {
                if let Ok(mut outbound) = TcpStream::connect(target).await {
                    let _ = copy_bidirectional(&mut inbound, &mut outbound).await;
                }
            });
        }
    }))
}
