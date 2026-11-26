use std::error::Error;
{% if context.mcp %}use std::sync::Arc;
{% endif %}{% if context.auth_enabled %}use std::time::Duration;
{% endif %}
use axum::{Router, http::Method};
{% if context.auth_enabled %}use baukit_auth::AuthState;
{% endif %}{% if context.mcp %}use baukit_auth::IdentityVerifier;
{% endif %}use baukit_config::{% if context.auth_enabled %}{BaukitConfig, Environment}{% else %}BaukitConfig{% endif %};
{% if context.auth_enabled %}use baukit_ratelimit::{RateLimitOptions, RedisRateLimitStore, SharedRateLimitStore};
{% else %}use baukit_ratelimit::{InMemoryRateLimitStore, RateLimitOptions, SharedRateLimitStore};
{% endif %}{% if context.auth_enabled or context.platform %}use sqlx::PgPool;
{% endif %}
use {{ context.app_crate }}_api::ApiState;
{% if not context.auth_enabled %}use {{ context.app_crate }}_api::finalize_api;
{% endif %}use {{ context.app_crate }}_api::routes;
{% if context.auth_enabled %}use {{ context.app_crate }}_api::{finalize_with_options, http_options};
{% endif %}
use {{ context.app_crate }}_services::ItemService;

{% if context.auth_enabled %}use crate::{BrowserSessionDependencies, browser_session_router, protect_routes};
{% endif %}{% if not context.auth_enabled or context.mcp or context.platform %}use crate::{ProductConfig, RateLimits, rate_limited};
{% else %}use crate::{ProductConfig, RateLimits};
{% endif %}{% if context.mcp %}use crate::{mcp_policy, mcp_verifier};
{% endif %}{% if context.auth_enabled %}
const HSTS_MAX_AGE: Duration = Duration::from_secs(365 * 24 * 60 * 60);
{% endif %}{% if context.auth_enabled %}const BROWSER_SESSION_ROUTES: [&str; 3] = [
    "/auth/session/exchange",
    "/auth/session/refresh",
    "/auth/session/logout",
];
{% endif %}{% if context.mcp %}const MCP_POST_ROUTES: [&str; 1] = ["/mcp"];
const MCP_GET_ROUTES: [&str; 2] = [
    "/.well-known/oauth-protected-resource",
    "/.well-known/oauth-protected-resource/mcp",
];
{% endif %}{% if context.platform %}const PLATFORM_INBOUND_ROUTE: &str = "/platform/inbound";
{% endif %}
/// Everything [`compose_api`] needs besides configuration.
#[derive(Clone)]
pub struct ApiDependencies {
{% if context.auth_enabled or context.platform %}    pub pool: PgPool,
{% endif %}    pub item_service: ItemService,
{% if context.auth_enabled %}    pub api_state: ApiState,
    pub auth: AuthState,
{% endif %}{% if context.mcp %}    pub verifier: Arc<dyn IdentityVerifier>,
{% endif %}    /// Replaces the configured rate-limit store. Tests pass an in-memory store here.
    pub rate_limit_store: Option<SharedRateLimitStore>,
}

/// The public API router plus the routes merged outside the OpenAPI document.
pub struct ComposedApi {
    pub router: Router,
    /// Every route a late router serves, as `(method, path)`.
    pub late_routes: Vec<(Method, &'static str)>,
}
{% if context.auth_enabled %}
/// Builds the public API with every layer the binary serves.
///
/// # Errors
///
/// Returns an error when rate-limit, HTTP, browser-session, MCP or platform configuration is invalid
/// or a backing service cannot be reached.
pub async fn compose_api(
    config: &BaukitConfig<ProductConfig>,
    deps: &ApiDependencies,
) -> Result<ComposedApi, Box<dyn Error>> {
    let rate_limit_options = RateLimitOptions::from_config(&config.rate_limit)?;
    let redis = match deps.rate_limit_store {
        Some(_) => None,
        None => RedisRateLimitStore::connect_if_enabled(&rate_limit_options).await?,
    };
    let store = deps
        .rate_limit_store
        .clone()
        .or_else(|| redis.clone().map(SharedRateLimitStore::new));
    let limits = store
        .filter(|_| rate_limit_options.is_enabled())
        .map(|store| RateLimits {
            store,
            options: rate_limit_options.clone(),
        });
    let mut http = http_options(&config.http, config.environment)?;
    if config.environment != Environment::Local {
        http = http.with_hsts(HSTS_MAX_AGE);
    }
    let mut late = Vec::new();
    let mut late_routes = Vec::new();
    if config.product.auth.browser_session.enabled {
        http.allow_credentials = true;
        let dependencies = BrowserSessionDependencies {
            issuer: &config.product.auth.issuer,
            http: &http,
            pool: deps.pool.clone(),
            rate_limit: &rate_limit_options,
            rate_limit_store: redis,
            environment: config.environment,
        };
        if let Some(router) =
            browser_session_router(&config.product.auth.browser_session, dependencies).await?
        {
            late.push(router);
            late_routes.extend(BROWSER_SESSION_ROUTES.map(|path| (Method::POST, path)));
        }
    }
{% if context.platform %}    let platform = crate::platform::compose(deps.pool.clone(), &config.product)?;
    if !config.product.platform.standalone() {
        late_routes.push((Method::POST, PLATFORM_INBOUND_ROUTE));
    }
{% endif %}    let api = protect_routes(
        routes(deps.api_state.clone()),
        late,
        deps.auth.clone(),
        limits.as_ref(),
    )?;
{% if context.mcp %}    // baukit_mcp limits verified callers itself; the IP limiter covers requests with invalid tokens.
    let mcp_store = mcp_store(config, limits.as_ref())?;
    let item_reads = Arc::new(deps.item_service.clone());
    let mcp_services = {{ context.app_crate }}_mcp::services(item_reads);
    let mcp = baukit_mcp::router(
        config.product.mcp.clone(),
        mcp_caller_verifier(config, deps).await?,
        mcp_services,
        mcp_store,
        mcp_policy(&config.product.auth, &config.product.mcp)?,
    )
    .await?;
    if config.product.mcp.enabled {
        late_routes.extend(MCP_POST_ROUTES.map(|path| (Method::POST, path)));
        late_routes.extend(MCP_GET_ROUTES.map(|path| (Method::GET, path)));
    }
    let api = api.merge(rate_limited(mcp, limits.as_ref()));
{% endif %}{% if context.platform %}    let api = api
        .merge(rate_limited(platform.router, limits.as_ref()))
        .layer(axum::Extension(platform.entitlements));
{% endif %}    Ok(ComposedApi {
        router: finalize_with_options(api, http),
        late_routes,
    })
}
{% if context.mcp %}
fn mcp_store(
    config: &BaukitConfig<ProductConfig>,
    limits: Option<&RateLimits>,
) -> Result<Arc<dyn baukit_ratelimit::RateLimitStore>, Box<dyn Error>> {
    match limits {
        Some(limits) => Ok(Arc::new(limits.store.clone())),
        None if config.environment == Environment::Local || !config.product.mcp.enabled => {
            Ok(Arc::new(baukit_ratelimit::InMemoryRateLimitStore::default()))
        }
        None => {
            Err("remote MCP requires a shared rate-limit store outside local development".into())
        }
    }
}

async fn mcp_caller_verifier(
    config: &BaukitConfig<ProductConfig>,
    deps: &ApiDependencies,
) -> Result<Arc<dyn IdentityVerifier>, Box<dyn Error>> {
    if config.product.mcp.enabled {
        return mcp_verifier(&config.product.auth, &config.product.mcp).await;
    }
    Ok(deps.verifier.clone())
}
{% endif %}{% else %}
/// Builds the public API with every layer the binary serves.
///
/// # Errors
///
/// Returns an error when rate-limit, HTTP or platform configuration is invalid.
pub fn compose_api(
    config: &BaukitConfig<ProductConfig>,
    deps: &ApiDependencies,
) -> Result<ComposedApi, Box<dyn Error>> {
    let options = RateLimitOptions::from_config(&config.rate_limit)?;
    // Without auth there is no Redis in the stack, so each replica keeps its own IP buckets.
    let store = deps
        .rate_limit_store
        .clone()
        .unwrap_or_else(|| SharedRateLimitStore::new(InMemoryRateLimitStore::default()));
    let limits = options.is_enabled().then(|| RateLimits { store, options });
    let api = routes(ApiState {
        items: deps.item_service.clone(),
    });
    let api = rate_limited(api, limits.as_ref());
{% if context.platform %}    let mut late_routes = Vec::new();
    let platform = crate::platform::compose(deps.pool.clone(), &config.product)?;
    if !config.product.platform.standalone() {
        late_routes.push((Method::POST, PLATFORM_INBOUND_ROUTE));
    }
    let api = api
        .merge(rate_limited(platform.router, limits.as_ref()))
        .layer(axum::Extension(platform.entitlements));
{% else %}    let late_routes = Vec::new();
{% endif %}    Ok(ComposedApi {
        router: finalize_api(api, &config.http, config.environment)?,
        late_routes,
    })
}
{% endif %}