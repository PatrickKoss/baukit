use std::{error::Error, time::Duration};

use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use baukit_auth::Principal;
use baukit_config::RateLimitConfig;
use baukit_ratelimit::{
    AuthenticatedRouteGroupOptions, InMemoryRateLimitStore, Quota, RateLimitOptions,
    RateLimitStore, authenticated_route_group,
};
use tower::ServiceExt;

#[tokio::test]
async fn rest_and_external_callers_consume_the_same_route_group_bucket()
-> Result<(), Box<dyn Error>> {
    let rate_limit = RateLimitOptions::from_config(&RateLimitConfig {
        key_prefix: "product:".into(),
        ..Default::default()
    })?;
    let quota = Quota::new(2, Duration::from_secs(3600), 0)?;
    let options = AuthenticatedRouteGroupOptions::new("writes", quota, &rate_limit)?;
    let key = options.key("account:alice");
    assert_eq!(key, "product:group:writes:account:alice");
    let store = InMemoryRateLimitStore::new(16)?;
    let app = authenticated_route_group(
        Router::new().route("/write", get(|| async { "written" })),
        store.clone(),
        options,
        |principal: &Principal| format!("account:{}", principal.subject()),
        |_| true,
    );
    let mut request = Request::builder().uri("/write").body(Body::empty())?;
    request.extensions_mut().insert(Principal::new("alice"));
    assert_eq!(app.clone().oneshot(request).await?.status(), StatusCode::OK);
    let decision = store.check_and_consume(&key, quota).await?;
    assert!(decision.allowed);
    assert_eq!(decision.remaining, 0);
    let mut request = Request::builder().uri("/write").body(Body::empty())?;
    request.extensions_mut().insert(Principal::new("alice"));
    assert_eq!(
        app.oneshot(request).await?.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    assert!(
        store
            .check_and_consume("product:group:writes:account:bob", quota)
            .await?
            .allowed
    );
    Ok(())
}
