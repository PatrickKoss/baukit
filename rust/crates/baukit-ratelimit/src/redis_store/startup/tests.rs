use std::io;

use baukit_test::start_redis;
use tokio::{
    io::copy_bidirectional,
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time::Instant,
};

use super::*;
use crate::{Quota, RateLimitStore as _};

const SHORT_BUDGET: Duration = Duration::from_secs(1);
const TEST_BUDGET: Duration = Duration::from_secs(60);
const LATE_START: Duration = Duration::from_secs(5);
const RECOVERY_WAIT: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_millis(200);

fn redis_error(error: RedisError) -> RateLimitStoreError {
    RateLimitStoreError::new(error)
}

#[test]
fn transient_redis_errors_are_retried() {
    for error in [
        RedisError::from(io::Error::from(io::ErrorKind::ConnectionRefused)),
        RedisError::from(io::Error::other("failed to lookup address information")),
        RedisError::from(io::Error::from(io::ErrorKind::TimedOut)),
        RedisError::from((ErrorKind::MasterNameNotFoundBySentinel, "no master")),
        RedisError::from((ErrorKind::NoValidReplicasFoundBySentinel, "no replica")),
    ] {
        assert!(is_transient(&redis_error(error.clone())), "{error}");
    }
}

#[test]
fn auth_and_configuration_errors_fail_fast() {
    for error in [
        RedisError::from((ErrorKind::AuthenticationFailed, "WRONGPASS")),
        RedisError::from((ErrorKind::InvalidClientConfig, "bad url")),
    ] {
        assert!(!is_transient(&redis_error(error.clone())), "{error}");
    }
    assert!(!is_transient(&RateLimitStoreError::unavailable("other")));
}

#[tokio::test]
async fn invalid_url_fails_without_retrying() {
    let started = Instant::now();
    let error = RedisRateLimitStore::connect_or_degraded("http://localhost", TEST_BUDGET)
        .await
        .expect_err("unsupported scheme");
    assert!(matches!(error, StartupError::Permanent(_)), "{error}");
    assert!(started.elapsed() < SHORT_BUDGET);
}

#[tokio::test]
async fn unreachable_redis_exhausts_the_budget() {
    let url = local_url(free_port().await);
    let error = RedisRateLimitStore::connect_with_backoff(&url, SHORT_BUDGET)
        .await
        .expect_err("nothing listens");
    assert!(error.is_budget_exhausted(), "{error}");
}

#[tokio::test]
async fn unreachable_redis_starts_a_degraded_store() {
    let url = local_url(free_port().await);
    let store = RedisRateLimitStore::connect_or_degraded(&url, SHORT_BUDGET)
        .await
        .expect("degraded store");
    assert!(!store.is_connected());
    assert!(format!("{store:?}").contains("connecting"));
    let quota = Quota::new(1, Duration::from_secs(1), 0).expect("quota");
    store
        .check_and_consume("key", quota)
        .await
        .expect_err("degraded store has no connection");
}

#[tokio::test]
#[ignore = "requires Docker Redis"]
async fn connects_when_redis_starts_after_the_connect_began() {
    let port = free_port().await;
    let started = Instant::now();
    let url = local_url(port);
    let connect =
        tokio::spawn(
            async move { RedisRateLimitStore::connect_with_backoff(&url, TEST_BUDGET).await },
        );

    let redis = start_redis().await.expect("start Redis");
    tokio::time::sleep(LATE_START.saturating_sub(started.elapsed())).await;
    let _proxy = forward(port, redis.connection_url()).await;

    let store = connect
        .await
        .expect("join")
        .expect("connect after late start");
    assert!(store.is_connected());
    assert!(started.elapsed() >= LATE_START);
}

#[tokio::test]
#[ignore = "requires Docker Redis"]
async fn degraded_store_recovers_when_redis_comes_back() {
    let port = free_port().await;
    let store = RedisRateLimitStore::connect_or_degraded(&local_url(port), SHORT_BUDGET)
        .await
        .expect("degraded store");
    let quota = Quota::new(10, Duration::from_secs(60), 0).expect("quota");
    store
        .check_and_consume("key", quota)
        .await
        .expect_err("still degraded");

    let redis = start_redis().await.expect("start Redis");
    let _proxy = forward(port, redis.connection_url()).await;

    let deadline = Instant::now() + RECOVERY_WAIT;
    while store.check_and_consume("key", quota).await.is_err() {
        assert!(Instant::now() < deadline, "store did not recover");
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(store.is_connected());
}

async fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    listener.local_addr().expect("address").port()
}

fn local_url(port: u16) -> String {
    format!("redis://127.0.0.1:{port}")
}

fn target_address(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let host = rest.rsplit_once('@').map_or(rest, |(_, host)| host);
    host.split('/').next().expect("host and port").to_owned()
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
