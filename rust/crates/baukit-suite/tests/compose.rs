use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode, header},
};
use baukit_ratelimit::{
    InMemoryRateLimitStore, Quota, RateLimitDecision, RateLimitStore, RateLimitStoreError,
};
use baukit_suite::{
    adapters::public_limit::SuitePublicRateLimit,
    compose::{PostgresSuite, PostgresSuiteBuilder, SuiteComposeError, SuiteSettings},
    config::SuiteConfig,
    domain::SuiteEvent,
    ports::{SuiteIdentity, SuiteIdentitySource, SuiteReplaySource, SuiteStoreError},
};
use sqlx::{PgConnection, PgPool};
use tower::ServiceExt as _;
use uuid::Uuid;

const PEERS: &str = include_str!("../../../../fixtures/suite-events/v1/peers.json");
const CATALOG: &str = include_str!("../../../../fixtures/suite-events/v1/catalog.json");
const APP_ID: &str = "alpha";
const PUBLIC_LIMIT: u64 = 3;

type TestResult = Result<(), Box<dyn std::error::Error>>;

struct NoIdentity;
#[async_trait]
impl SuiteIdentitySource for NoIdentity {
    async fn identity(&self, _owner: Uuid) -> Result<SuiteIdentity, SuiteStoreError> {
        Err(SuiteStoreError::NotFound)
    }
    async fn identity_in_transaction(
        &self,
        _connection: &mut PgConnection,
        _owner: Uuid,
    ) -> Result<SuiteIdentity, SuiteStoreError> {
        Err(SuiteStoreError::NotFound)
    }
}

struct NoReplay;
#[async_trait]
impl SuiteReplaySource for NoReplay {
    async fn replay_since(
        &self,
        _connection: &mut PgConnection,
        _owner: Uuid,
        _since: chrono::NaiveDate,
    ) -> Result<Vec<SuiteEvent>, SuiteStoreError> {
        Ok(Vec::new())
    }
}

struct UnavailableStore;
impl RateLimitStore for UnavailableStore {
    fn check_and_consume<'a>(
        &'a self,
        _key: &'a str,
        _quota: Quota,
    ) -> std::pin::Pin<
        Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>,
    > {
        Box::pin(async {
            Err(RateLimitStoreError::new(std::io::Error::other(
                "store unavailable",
            )))
        })
    }
}

fn builder_with(
    limiter: Arc<dyn RateLimitStore>,
) -> Result<PostgresSuiteBuilder, Box<dyn std::error::Error>> {
    let pool = PgPool::connect_lazy("postgres://suite:suite@127.0.0.1:1/suite")?;
    let settings = SuiteSettings::new(APP_ID, PEERS, CATALOG, &SuiteConfig::default(), None)?;
    Ok(PostgresSuite::builder(pool, settings, limiter))
}

fn builder() -> Result<PostgresSuiteBuilder, Box<dyn std::error::Error>> {
    builder_with(Arc::new(InMemoryRateLimitStore::default()))
}

fn suite() -> Result<PostgresSuite, Box<dyn std::error::Error>> {
    let limit = SuitePublicRateLimit::new()?.with_quota(Quota::new(
        PUBLIC_LIMIT,
        Duration::from_secs(60),
        0,
    )?);
    Ok(builder()?
        .identity(Arc::new(NoIdentity))
        .replay(Arc::new(NoReplay))
        .public_rate_limit(limit)
        .build()?)
}

fn post(uri: &str) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
}

async fn statuses(
    router: &axum::Router,
    request: impl Fn() -> Result<Request<Body>, axum::http::Error>,
    count: u64,
) -> Result<Vec<StatusCode>, Box<dyn std::error::Error>> {
    let mut statuses = Vec::new();
    for _ in 0..count {
        statuses.push(router.clone().oneshot(request()?).await?.status());
    }
    Ok(statuses)
}

#[tokio::test]
async fn hammering_exchange_returns_429_with_retry_after() -> TestResult {
    let suite = suite()?;
    let router = suite.router(suite.api());
    let statuses = statuses(&router, || post("/suite/links/exchange"), PUBLIC_LIMIT).await?;
    assert!(
        statuses
            .iter()
            .all(|status| *status != StatusCode::TOO_MANY_REQUESTS)
    );
    let limited = router.oneshot(post("/suite/links/exchange")?).await?;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(limited.headers().contains_key(header::RETRY_AFTER));
    Ok(())
}

#[tokio::test]
async fn unavailable_limiter_rejects_public_routes_by_default() -> TestResult {
    let suite = builder_with(Arc::new(UnavailableStore))?
        .identity(Arc::new(NoIdentity))
        .replay(Arc::new(NoReplay))
        .build()?;
    let response = suite
        .router(suite.api())
        .oneshot(post("/suite/links/exchange")?)
        .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    Ok(())
}

#[tokio::test]
async fn callback_and_revoke_share_the_public_ip_bucket() -> TestResult {
    let suite = suite()?;
    let router = suite.router(suite.api());
    let revoke = format!("/suite/links/{}/revoke", Uuid::nil());
    statuses(&router, || post(&revoke), PUBLIC_LIMIT).await?;
    let callback = Request::get("/suite/links/callback").body(Body::empty())?;
    assert_eq!(
        router.oneshot(callback).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    Ok(())
}

#[tokio::test]
async fn authenticated_and_inbound_routes_skip_the_public_ip_bucket() -> TestResult {
    let suite = suite()?;
    let router = suite.router(suite.api());
    let attempts = PUBLIC_LIMIT * 3;
    let inbound = format!("/suite/inbound/{}", Uuid::nil());
    let inbound = statuses(&router, || post(&inbound), attempts).await?;
    let user = statuses(&router, || post("/suite/authorizations/deny"), attempts).await?;
    assert!(
        inbound
            .iter()
            .chain(&user)
            .all(|status| *status != StatusCode::TOO_MANY_REQUESTS)
    );
    Ok(())
}

#[tokio::test]
async fn build_requires_identity_and_replay() -> TestResult {
    let missing = builder()?.replay(Arc::new(NoReplay)).build();
    assert!(matches!(
        missing,
        Err(SuiteComposeError::Missing("identity"))
    ));
    let missing = builder()?.identity(Arc::new(NoIdentity)).build();
    assert!(matches!(missing, Err(SuiteComposeError::Missing("replay"))));
    Ok(())
}

#[tokio::test]
async fn build_takes_share_xp_from_config_unless_overridden() -> TestResult {
    let configured = builder()?
        .identity(Arc::new(NoIdentity))
        .replay(Arc::new(NoReplay))
        .build()?;
    assert!(configured.context.share_xp);
    assert_eq!(configured.context.metric_prefix, APP_ID);
    let overridden = builder()?
        .identity(Arc::new(NoIdentity))
        .replay(Arc::new(NoReplay))
        .share_xp(false)
        .build()?;
    assert!(!overridden.context.share_xp);
    assert!(overridden.erasure.is_none() && overridden.notifier().is_none());
    Ok(())
}
