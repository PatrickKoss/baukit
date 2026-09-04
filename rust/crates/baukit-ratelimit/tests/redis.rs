use std::{
    error::Error,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime},
};

use axum::{
    Extension, Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode, header::RETRY_AFTER},
    routing::get,
};
use baukit_auth::Principal;
use baukit_ratelimit::{
    AmountBudgetStore as _, AuthenticatedRouteGroupOptions, InMemoryRateLimitStore, Quota,
    RATE_LIMIT_LIMIT, RATE_LIMIT_REMAINING, RateLimitOptions, RateLimitStore, RedisRateLimitStore,
    authenticated_route_group, layers,
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
async fn concurrent_amount_consumers_never_exceed_the_fixed_window_limit()
-> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = Arc::new(RedisRateLimitStore::connect(fixture.connection_url()).await?);
    let now = SystemTime::now();
    let reset_at = now + Duration::from_secs(3_600);
    let barrier = Arc::new(Barrier::new(51));
    let mut tasks = Vec::new();
    for _ in 0..50 {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            store
                .check_and_consume_amount("amount-budget:test:atomic:subject", 3, 30, now, reset_at)
                .await
        }));
    }
    barrier.wait().await;

    let mut admitted = 0;
    for task in tasks {
        admitted += usize::from(task.await??.allowed);
    }
    assert_eq!(admitted, 10);

    let final_decision = store
        .check_and_consume_amount("amount-budget:test:atomic:subject", 1, 30, now, reset_at)
        .await?;
    assert!(!final_decision.allowed);
    assert_eq!(final_decision.remaining, 0);

    for (suffix, amount, allowed, remaining) in [
        ("below", 29, true, 1),
        ("at", 30, true, 0),
        ("above", 31, false, 30),
    ] {
        let decision = store
            .check_and_consume_amount(
                &format!("amount-budget:test:{suffix}:subject"),
                amount,
                30,
                now,
                reset_at,
            )
            .await?;
        assert_eq!(decision.allowed, allowed, "amount {amount}");
        assert_eq!(decision.remaining, remaining, "amount {amount}");
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn redis_amount_counter_expires_at_the_supplied_boundary() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let now = SystemTime::now();
    let reset_after = Duration::from_secs(5);
    let reset_at = now + reset_after;
    let key = "amount-budget:test:expiry:subject";
    let decision = store
        .check_and_consume_amount(key, 4, 10, now, reset_at)
        .await?;
    assert!(decision.allowed);
    assert_eq!(decision.remaining, 6);

    let client = redis::Client::open(fixture.connection_url())?;
    let mut connection = client.get_multiplexed_async_connection().await?;
    let ttl: i64 = connection.pttl(key).await?;
    let maximum_ttl = i64::try_from(reset_after.as_millis())? + 1;
    assert!(ttl > 0 && ttl <= maximum_ttl, "unexpected TTL: {ttl}");
    tokio::time::sleep(reset_after + Duration::from_secs(1)).await;
    let exists: bool = connection.exists(key).await?;
    assert!(!exists);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn redis_amount_release_is_floored_and_keeps_the_window_expiry() -> Result<(), Box<dyn Error>>
{
    let fixture = baukit_test::start_redis().await?;
    let store = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let now = SystemTime::now();
    let reset_after = Duration::from_secs(30);
    let reset_at = now + reset_after;
    let key = "amount-budget:test:release:subject";
    store
        .check_and_consume_amount(key, 7, 10, now, reset_at)
        .await?;

    let partial = store.release_amount(key, 3, 10, now, reset_at).await?;
    assert!(partial.allowed);
    assert_eq!(partial.remaining, 6);

    let floored = store.release_amount(key, 20, 10, now, reset_at).await?;
    assert!(floored.allowed);
    assert_eq!(floored.remaining, 10);

    let client = redis::Client::open(fixture.connection_url())?;
    let mut connection = client.get_multiplexed_async_connection().await?;
    let consumed: u64 = connection.get(key).await?;
    assert_eq!(consumed, 0);
    let ttl: i64 = connection.pttl(key).await?;
    let maximum_ttl = i64::try_from(reset_after.as_millis())? + 1;
    assert!(ttl > 0 && ttl <= maximum_ttl, "unexpected TTL: {ttl}");

    let new_window_key = "amount-budget:test:release-next-window:subject";
    let new_window = store
        .release_amount(new_window_key, 7, 10, now, reset_at)
        .await?;
    assert!(new_window.allowed);
    assert_eq!(new_window.remaining, 10);
    let exists: bool = connection.exists(new_window_key).await?;
    assert!(!exists);
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
    // Wait comfortably past the 200 ms TTL; overshooting only strengthens the
    // assertion that the key is gone.
    tokio::time::sleep(Duration::from_millis(600)).await;
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
    // One token accrues every 100 ms at this quota. Sleeping only just past that left
    // a ~20 ms margin, which a loaded host eats: `last_refill_ms` is stamped by the
    // denied call above, so any stall before the sleep starts shortens it in effect.
    tokio::time::sleep(Duration::from_millis(400)).await;
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

    let original_master = fixture.master_address().await?;
    fixture.stop_master().await?;

    // Wait for Sentinel to publish the promoted replica rather than polling the
    // store blindly. Sentinel keeps serving the dead master's address for a moment
    // after the replica promotes itself, so a store that resolves inside that window
    // reconnects to the node that just died.
    fixture.wait_for_failover(&original_master).await?;

    // The store still has to notice the new master and re-resolve, which can take a
    // few attempts after Sentinel has already published the promotion.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
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

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn memory_and_redis_route_groups_make_the_same_decisions() -> Result<(), Box<dyn Error>> {
    let fixture = baukit_test::start_redis().await?;
    let redis = RedisRateLimitStore::connect(fixture.connection_url()).await?;
    let memory = route_group_statuses(InMemoryRateLimitStore::default()).await?;
    let redis = route_group_statuses(redis).await?;

    assert_eq!(memory, redis);
    assert_eq!(redis, [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS]);
    Ok(())
}

async fn route_group_statuses(
    store: impl RateLimitStore + 'static,
) -> Result<[StatusCode; 2], Box<dyn Error>> {
    let quota = Quota::new(1, Duration::from_secs(3_600), 0)?;
    let options =
        AuthenticatedRouteGroupOptions::new("redis_parity", quota, &RateLimitOptions::default())?;
    let app = authenticated_route_group(
        Router::new().route("/", get(|| async { "ok" })),
        store,
        options,
        |principal: &Principal| principal.subject().to_owned(),
        |_| true,
    );
    let first = app.clone().oneshot(group_request()).await?.status();
    let second = app.oneshot(group_request()).await?.status();
    Ok([first, second])
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

fn group_request() -> Request<Body> {
    let mut request = Request::builder()
        .uri("/")
        .body(Body::empty())
        .expect("request");
    request.extensions_mut().insert(Principal::new("subject"));
    request
}
