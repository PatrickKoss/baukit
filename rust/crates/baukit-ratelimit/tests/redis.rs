use std::{error::Error, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Extension, Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header::RETRY_AFTER},
    routing::get,
};
use baukit_auth::Principal;
use baukit_ratelimit::{
    Quota, RATE_LIMIT_LIMIT, RATE_LIMIT_REMAINING, RateLimitOptions, RateLimitStore as _,
    RedisRateLimitStore, layers,
};
use redis::AsyncCommands as _;
use tokio::sync::Barrier;
use tower::ServiceExt as _;

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_redis_consumption_never_over_admits() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = Arc::new(RedisRateLimitStore::connect(fixture.connection_url()).await?);
    let quota = Quota::new(10, Duration::from_secs(3_600), 0)?;
    let barrier = Arc::new(Barrier::new(51));
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store.check_and_consume("rl:test:atomic", quota).await
        }));
    }
    barrier.wait().await;

    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await??.allowed);
    }
    assert_eq!(admitted, 10);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn redis_idle_keys_expire() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let quota = Quota::new(1, Duration::from_millis(200), 0)?;
    assert!(store.check_and_consume("rl:test:ttl", quota).await?.allowed);

    let client = redis::Client::open(fixture.connection_url())?;
    let mut connection = client.get_multiplexed_async_connection().await?;
    let ttl: i64 = connection.pttl("rl:test:ttl").await?;
    assert!(ttl > 0 && ttl <= 200, "unexpected TTL: {ttl}");
    tokio::time::sleep(Duration::from_millis(300)).await;
    let exists: bool = connection.exists("rl:test:ttl").await?;
    assert!(!exists);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn redis_bucket_refills_over_time() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let quota = Quota::new(2, Duration::from_millis(200), 0)?;
    assert!(
        store
            .check_and_consume("rl:test:refill", quota)
            .await?
            .allowed
    );
    assert!(
        store
            .check_and_consume("rl:test:refill", quota)
            .await?
            .allowed
    );
    assert!(
        !store
            .check_and_consume("rl:test:refill", quota)
            .await?
            .allowed
    );
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert!(
        store
            .check_and_consume("rl:test:refill", quota)
            .await?
            .allowed
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn sentinel_url_enforces_quota() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis_sentinel().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let quota = Quota::new(2, Duration::from_secs(60), 0)?;

    let first = store
        .check_and_consume("rl:test:sentinel:quota", quota)
        .await?;
    assert!(first.allowed);
    assert_eq!(first.remaining, 1);
    assert_eq!(first.retry_after, Duration::ZERO);

    let second = store
        .check_and_consume("rl:test:sentinel:quota", quota)
        .await?;
    assert!(second.allowed);
    assert_eq!(second.remaining, 0);

    let limited = store
        .check_and_consume("rl:test:sentinel:quota", quota)
        .await?;
    assert!(!limited.allowed);
    assert_eq!(limited.remaining, 0);
    assert!(limited.retry_after > Duration::ZERO);
    assert!(limited.retry_after <= Duration::from_secs(30));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn sentinel_store_recovers_after_master_failover() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis_sentinel().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let probe_quota = Quota::new(100, Duration::from_secs(60), 0)?;
    assert!(
        store
            .check_and_consume("rl:test:sentinel:initial", probe_quota)
            .await?
            .allowed
    );

    fixture.stop_master().await?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match store
            .check_and_consume("rl:test:sentinel:recovery-probe", probe_quota)
            .await
        {
            Ok(decision) if decision.allowed => break,
            Ok(_) | Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
            Ok(decision) => panic!("store did not admit after failover: {decision:?}"),
            Err(error) => {
                return Err(format!("store did not recover after failover: {error}").into());
            }
        }
    }

    let quota = Quota::new(1, Duration::from_secs(60), 0)?;
    let allowed = store
        .check_and_consume("rl:test:sentinel:post-failover", quota)
        .await?;
    let limited = store
        .check_and_consume("rl:test:sentinel:post-failover", quota)
        .await?;
    assert!(allowed.allowed);
    assert_eq!(allowed.remaining, 0);
    assert!(!limited.allowed);
    assert_eq!(limited.remaining, 0);
    assert!(limited.retry_after > Duration::ZERO);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn real_redis_works_through_full_axum_layer() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let mut options = RateLimitOptions::default();
    options.identity.quota = Quota::new(1, Duration::from_secs(60), 0)?;
    options.ip.quota = Quota::new(1_000, Duration::from_secs(60), 0)?;
    let app = layers(
        Router::new().route("/", get(|| async { "ok" })),
        store,
        options,
    )
    .layer(Extension(Principal::new("redis-fixture-subject")));

    let first = app.clone().oneshot(request()).await?;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()[RATE_LIMIT_LIMIT], "1");
    assert_eq!(first.headers()[RATE_LIMIT_REMAINING], "0");

    let second = app.oneshot(request()).await?;
    assert_eq!(second.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(second.headers().contains_key(RETRY_AFTER));
    assert_eq!(second.headers()[RATE_LIMIT_LIMIT], "1");
    Ok(())
}

fn request() -> Request<Body> {
    let mut request = Request::builder()
        .uri("/")
        .body(Body::empty())
        .expect("request");
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.1:1234".parse::<SocketAddr>().expect("peer"),
    ));
    request
}
