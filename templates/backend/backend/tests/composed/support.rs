{% if context.auth_enabled %}use std::sync::Arc;
{% endif %}use std::{error::Error, net::SocketAddr, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Method, Request, StatusCode, header},
};
{% if context.auth_enabled %}use baukit_auth::{AuthState, IdentityVerifier, OidcConfig, OidcVerifier};
{% endif %}use baukit_config::{BaukitConfig, RateLimitScopeConfig};
use baukit_ratelimit::{InMemoryRateLimitStore, SharedRateLimitStore};
use serde_json::Value;
use tower::ServiceExt as _;

{% if context.auth_enabled %}use {{ context.app_crate }}_api::ApiState;
use {{ context.app_crate }}_api::ErasureApi;
{% endif %}use {{ context.app_crate }}_bin::ApiDependencies;
use {{ context.app_crate }}_bin::ComposedApi;
use {{ context.app_crate }}_bin::InMemoryItemRepository;
{% if context.auth_enabled %}use {{ context.app_crate }}_bin::InMemoryUserRepository;
{% endif %}use {{ context.app_crate }}_bin::ProductConfig;
use {{ context.app_crate }}_bin::compose_api;
{% if context.auth_enabled %}use {{ context.app_crate }}_postgres::PostgresProfileErasure;
{% endif %}use {{ context.app_crate }}_services::ItemService;
{% if context.auth_enabled %}use {{ context.app_crate }}_services::UserService;
{% endif %}
{% if context.auth_enabled %}const WEB_ORIGIN: &str = "http://localhost:5173";
pub const AUDIENCE: &str = "{{ context.app_name }}-backend";
{% endif %}pub const BODY_LIMIT: usize = 1024;
const RESPONSE_LIMIT: usize = 1024 * 1024;
const PERIOD: Duration = Duration::from_secs(60);
{% if context.auth_enabled or context.platform %}const UNREACHABLE_DATABASE: &str = "postgres://unused:unused@127.0.0.1:1/unused";
{% endif %}
/// Routes merged outside the OpenAPI document. A late router missing here fails G-03.
pub const LATE_ROUTES: [(&str, &str); 7] = [
    ("POST", "/auth/session/exchange"),
    ("POST", "/auth/session/refresh"),
    ("POST", "/auth/session/logout"),
    ("POST", "/mcp"),
    ("GET", "/.well-known/oauth-protected-resource"),
    ("GET", "/.well-known/oauth-protected-resource/mcp"),
    ("POST", "/platform/inbound"),
];

pub struct Quota {
    pub requests: u64,
    pub burst: u64,
}

impl Quota {
    pub const ONE: Self = Self {
        requests: 1,
        burst: 0,
    };
    pub const GENEROUS: Self = Self {
        requests: 100_000,
        burst: 100_000,
    };
}

pub struct Composed {
    pub api: ComposedApi,
{% if context.auth_enabled %}    pub issuer: baukit_test::MockOidcServer,
{% endif %}}

pub fn config(quota: &Quota) -> BaukitConfig<ProductConfig> {
    let mut config = BaukitConfig::<ProductConfig>::default();
    let scope = RateLimitScopeConfig {
        enabled: true,
        requests_per_period: quota.requests,
        period: PERIOD,
        burst: quota.burst,
    };
    config.rate_limit.identity = scope.clone();
    config.rate_limit.ip = scope;
    config.http.body_size_limit = BODY_LIMIT;
{% if context.auth_enabled %}    config.http.cors_allowed_origins = vec![WEB_ORIGIN.to_owned()];
{% endif %}    config
}
{% if context.auth_enabled %}
/// Composes the API against a mock issuer. `erasure` defaults to a store on an unreachable database.
pub async fn compose(
    quota: &Quota,
    erasure: Option<ErasureApi>,
) -> Result<Composed, Box<dyn Error>> {
    let issuer = baukit_test::MockOidcServer::start().await?;
    let verifier: Arc<dyn IdentityVerifier> =
        Arc::new(OidcVerifier::discover(OidcConfig::new(issuer.issuer(), AUDIENCE)?).await?);
    compose_with(quota, erasure, issuer, verifier).await
}

pub async fn compose_with(
    quota: &Quota,
    erasure: Option<ErasureApi>,
    issuer: baukit_test::MockOidcServer,
    verifier: Arc<dyn IdentityVerifier>,
) -> Result<Composed, Box<dyn Error>> {
    let mut config = config(quota);
    config.product.auth.issuer = issuer.issuer().to_owned();
    config.product.auth.audience = AUDIENCE.to_owned();
{% if context.auth_oidc %}    config.product.auth.browser_session.enabled = true;
{% endif %}{% if context.mcp and context.auth_oidc %}    config.product.mcp.enabled = true;
    config.product.mcp.issuer = issuer.issuer().to_owned();
    config.product.mcp.resource_url = "http://localhost/mcp".to_owned();
    config.product.mcp.allowed_hosts = vec!["localhost".to_owned()];
{% endif %}    let pool = sqlx::PgPool::connect_lazy(UNREACHABLE_DATABASE)?;
    let erasure = match erasure {
        Some(erasure) => erasure,
        None => unreachable_erasure(pool.clone())?,
    };
    let items = ItemService::new(Arc::new(InMemoryItemRepository::new()));
    let deps = ApiDependencies {
        pool,
        item_service: items.clone(),
        api_state: ApiState {
            items,
            users: UserService::new(Arc::new(InMemoryUserRepository::new())),
            auth: AuthState::from_shared(verifier.clone()),
            erasure,
        },
        auth: AuthState::from_shared(verifier{% if context.mcp %}.clone(){% endif %}),
{% if context.mcp %}        verifier,
{% endif %}        rate_limit_store: Some(SharedRateLimitStore::new(InMemoryRateLimitStore::default())),
    };
    Ok(Composed {
        api: compose_api(&config, &deps).await?,
        issuer,
    })
}

fn unreachable_erasure(pool: sqlx::PgPool) -> Result<ErasureApi, Box<dyn Error>> {
    let store = baukit_erasure::PostgresErasureStore::new(
        pool,
        baukit_config::Secret::new("test-hash-key-with-at-least-32-bytes".into()),
    )?;
    Ok(ErasureApi {
        service: baukit_erasure::ErasureService::new(
            store,
            baukit_erasure::IdentityRetention::Delete {
                deleter: Arc::new(baukit_test::FakeIdentityAccountDeleter::default()),
                provider_id: "keycloak".into(),
                inline_timeout: Duration::from_secs(1),
                max_attempts: 3,
            },
        )?,
        product: Arc::new(PostgresProfileErasure),
    })
}
{% else %}
pub fn compose(quota: &Quota) -> Result<Composed, Box<dyn Error>> {
    let config = config(quota);
    let deps = ApiDependencies {
{% if context.platform %}        pool: sqlx::PgPool::connect_lazy(UNREACHABLE_DATABASE)?,
{% endif %}        item_service: ItemService::new(std::sync::Arc::new(InMemoryItemRepository::new())),
        rate_limit_store: Some(SharedRateLimitStore::new(InMemoryRateLimitStore::default())),
    };
    Ok(Composed {
        api: compose_api(&config, &deps)?,
    })
}
{% endif %}
pub fn openapi() -> Result<Value, Box<dyn Error>> {
    let document = {{ context.app_crate }}_api::openapi_document();
    Ok(serde_json::to_value(document)?)
}

/// Every `(METHOD, path)` the composed router serves, with path parameters replaced by a nil UUID.
pub fn inventory(composed: &ComposedApi) -> Result<Vec<(Method, String)>, Box<dyn Error>> {
    let document = openapi()?;
    let paths = document
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("the OpenAPI document has no paths")?;
    let mut routes = Vec::new();
    for (path, item) in paths {
        let Some(item) = item.as_object() else {
            continue;
        };
        for method in item.keys() {
            routes.push((
                Method::from_bytes(method.to_uppercase().as_bytes())?,
                concrete(path),
            ));
        }
    }
    for (method, path) in &composed.late_routes {
        routes.push((method.clone(), (*path).to_owned()));
    }
    Ok(routes)
}

pub fn assert_late_routes_listed(composed: &ComposedApi) {
    for (method, path) in &composed.late_routes {
        assert!(
            LATE_ROUTES.contains(&(method.as_str(), *path)),
            "late route {method} {path} is not in LATE_ROUTES; add it so G-03 covers it"
        );
    }
}

{% if context.auth_enabled %}/// OpenAPI path templates. The template documents only bearer-secured operations.
pub fn secured_paths() -> Result<Vec<String>, Box<dyn Error>> {
    Ok(openapi()?
        .get("paths")
        .and_then(Value::as_object)
        .ok_or("the OpenAPI document has no paths")?
        .keys()
        .cloned()
        .collect())
}

pub fn is_secured(templates: &[String], path: &str) -> bool {
    let path = path.split('?').next().unwrap_or_default();
    templates.iter().any(|template| {
        let expected: Vec<&str> = template.split('/').collect();
        let actual: Vec<&str> = path.split('/').collect();
        expected.len() == actual.len()
            && expected
                .iter()
                .zip(&actual)
                .all(|(expected, actual)| expected.starts_with('{') || expected == actual)
    })
}

/// Bearer tokens a verifier must reject: random kid, `alg: none`, HS256 confusion, expired.
pub fn hostile_tokens(issuer: &baukit_test::MockOidcServer) -> Result<Vec<String>, Box<dyn Error>> {
    let claims = issuer.claims("attacker", AUDIENCE, Duration::from_secs(60))?;
    let expired = claims.clone().expires_at(1);
    Ok(vec![
        issuer.mint_with_key_id(&claims, "random-kid")?,
        baukit_test::unsigned_token(&claims)?,
        baukit_test::hs256_token(b"guessed-secret", &claims)?,
        issuer.mint(&expired)?,
    ])
}

{% endif %}fn concrete(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            if segment.starts_with('{') {
                "00000000-0000-0000-0000-000000000000"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

pub struct Reply {
    pub status: StatusCode,
    pub headers: HeaderMap,
}

pub fn request(
    method: Method,
    path: &str,
    peer: SocketAddr,
    authorization: Option<&str>,
    body: Vec<u8>,
) -> Result<Request<Body>, Box<dyn Error>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, "localhost")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(authorization) = authorization {
        builder = builder.header(header::AUTHORIZATION, authorization);
    }
    if !body.is_empty() {
        builder = builder.header(header::CONTENT_LENGTH, body.len());
    }
    let mut request = builder.body(Body::from(body))?;
    request.extensions_mut().insert(ConnectInfo(peer));
    Ok(request)
}

pub async fn send(router: &Router, request: Request<Body>) -> Result<Reply, Box<dyn Error>> {
    let response = router.clone().oneshot(request).await?;
    let status = response.status();
    let headers = response.headers().clone();
    to_bytes(response.into_body(), RESPONSE_LIMIT).await?;
    Ok(Reply { status, headers })
}

pub fn assert_security_headers(label: &str, headers: &HeaderMap) {
    assert_eq!(
        headers
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .and_then(|value| value.to_str().ok()),
        Some("nosniff"),
        "{label}: missing X-Content-Type-Options: nosniff"
    );
    assert!(
        headers.contains_key(header::CONTENT_SECURITY_POLICY),
        "{label}: missing Content-Security-Policy"
    );
}
