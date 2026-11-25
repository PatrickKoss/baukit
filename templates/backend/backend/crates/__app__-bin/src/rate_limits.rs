{% if context.auth_enabled %}use std::{error::Error, time::Duration};

use axum::{Router, extract::Request, http::Method, middleware};
use baukit_auth::{AuthState, Principal};
use baukit_ratelimit::{
    AuthenticatedRouteGroupOptions, Quota, RateLimitOptions, SharedRateLimitStore,
};

const ITEM_WRITE_GROUP: &str = "item_writes";
const ITEM_WRITE_REQUESTS_PER_MINUTE: u64 = 30;
const ITEM_WRITE_PERIOD: Duration = Duration::from_secs(60);
{% else %}use axum::Router;
use baukit_ratelimit::{RateLimitOptions, SharedRateLimitStore};
{% endif %}
/// The store and quotas every public route is limited with.
#[derive(Clone)]
pub struct RateLimits {
    pub store: SharedRateLimitStore,
    pub options: RateLimitOptions,
}

/// Wraps `router` in the IP and identity limiter. `None` means rate limiting is disabled.
pub fn rate_limited(router: Router, limits: Option<&RateLimits>) -> Router {
    match limits {
        Some(limits) => {
            baukit_ratelimit::layers(router, limits.store.clone(), limits.options.clone())
        }
        None => router,
    }
}{% if context.auth_enabled %}

/// Authenticates and limits `items`, then merges each router in `late` behind its own limiter.
///
/// The IP limiter runs before `establish_principal`, so junk bearers are throttled per client
/// before any token verification. The identity limiter runs after it.
///
/// Routers in `late` verify callers themselves (MCP, browser sessions, signed webhooks), so
/// they skip `establish_principal` and fall into the IP bucket.
///
/// # Errors
///
/// Returns an error when the item-write quota is invalid for the configured options.
pub fn protect_routes(
    items: Router,
    late: impl IntoIterator<Item = Router>,
    auth: AuthState,
    limits: Option<&RateLimits>,
) -> Result<Router, Box<dyn Error>> {
    let items = match limits {
        Some(limits) => baukit_ratelimit::authenticated_route_group(
            items,
            limits.store.clone(),
            item_write_options(&limits.options)?,
            item_write_subject,
            is_item_write,
        ),
        None => items,
    };
    // Axum runs the last added layer first: IP pre-limit, then auth, then the identity limiter.
    let identity = limits.map(|limits| only_scope(limits, Scope::Identity));
    let ip = limits.map(|limits| only_scope(limits, Scope::Ip));
    let items = rate_limited(items, identity.as_ref()).layer(middleware::from_fn_with_state(
        auth,
        baukit_auth::establish_principal,
    ));
    let items = rate_limited(items, ip.as_ref());
    Ok(late.into_iter().fold(items, |router, late| {
        router.merge(rate_limited(late, limits))
    }))
}

#[derive(Clone, Copy)]
enum Scope {
    Identity,
    Ip,
}

fn only_scope(limits: &RateLimits, scope: Scope) -> RateLimits {
    let mut options = limits.options.clone();
    match scope {
        Scope::Identity => options.ip.enabled = false,
        Scope::Ip => options.identity.enabled = false,
    }
    RateLimits {
        store: limits.store.clone(),
        options,
    }
}

fn item_write_options(
    options: &RateLimitOptions,
) -> Result<AuthenticatedRouteGroupOptions, Box<dyn Error>> {
    let quota = Quota::new(ITEM_WRITE_REQUESTS_PER_MINUTE, ITEM_WRITE_PERIOD, 0)?;
    Ok(AuthenticatedRouteGroupOptions::new(
        ITEM_WRITE_GROUP,
        quota,
        options,
    )?)
}

fn item_write_subject(principal: &Principal) -> String {
    principal.subject().to_owned()
}

fn is_item_write(request: &Request) -> bool {
    matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}{% endif %}
