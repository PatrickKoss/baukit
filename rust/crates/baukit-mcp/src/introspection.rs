use std::{
    collections::BTreeMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_lc_rs::digest;
use baukit_config::Secret;
use reqwest::Client;
use serde::Deserialize;
use tokio::{sync::Mutex, time::Instant};

use crate::{
    AuthenticationPolicy, McpConfigError, PolicyDenial, PolicyFuture, Principal, VerifiedPrincipal,
};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2);
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5);
const DEFAULT_CACHE_CAPACITY: usize = 1024;

const MAX_CACHE_TTL: Duration = Duration::from_secs(30);
const MAX_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_CACHE_CAPACITY: usize = 4096;
const MAX_RESPONSE_BYTES: usize = 32 * 1024;

fn encode_client_credential(value: &str) -> String {
    let mut encoded = url::form_urlencoded::Serializer::new(String::new());
    encoded.append_key_only(value);
    encoded.finish()
}

/// Credentials for a separate confidential Keycloak resource-server client.
pub struct KeycloakIntrospectionConfig {
    pub issuer: String,
    pub client_id: String,
    pub client_secret: Secret<String>,
    pub timeout: Duration,
    /// Zero disables caching for per-request revocation checks. Maximum: 30 seconds.
    pub cache_ttl: Duration,
    pub cache_capacity: usize,
}

impl KeycloakIntrospectionConfig {
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: Secret<String>,
    ) -> Self {
        Self {
            issuer: issuer.into(),
            client_id: client_id.into(),
            client_secret,
            timeout: DEFAULT_TIMEOUT,
            cache_ttl: DEFAULT_CACHE_TTL,
            cache_capacity: DEFAULT_CACHE_CAPACITY,
        }
    }
}

/// RFC 7662 introspection with bounded successful-result caching and no stale fallback.
pub struct KeycloakIntrospectionPolicy {
    config: KeycloakIntrospectionConfig,
    endpoint: String,
    client: Client,
    cache: Mutex<BTreeMap<[u8; 32], CacheEntry>>,
}

#[derive(Clone, Deserialize)]
struct Introspection {
    active: bool,
    sub: Option<String>,
    iss: Option<String>,
    client_id: Option<String>,
    scope: Option<String>,
    exp: Option<u64>,
}

struct CacheEntry {
    expires_at: Instant,
    result: Introspection,
}

impl KeycloakIntrospectionPolicy {
    pub fn new(config: KeycloakIntrospectionConfig) -> Result<Self, McpConfigError> {
        crate::config::endpoint(&config.issuer)?;
        if config.client_id.is_empty()
            || config.client_secret.expose().is_empty()
            || config.timeout.is_zero()
            || config.timeout > MAX_TIMEOUT
            || config.cache_ttl > MAX_CACHE_TTL
            || config.cache_capacity == 0
            || config.cache_capacity > MAX_CACHE_CAPACITY
        {
            return Err(McpConfigError::Invalid(
                "introspection requires credentials, timeout in (0, 30s], cache TTL in [0, 30s], and capacity in [1, 4096]",
            ));
        }
        let client = Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| McpConfigError::Invalid("cannot build introspection client"))?;
        Ok(Self {
            endpoint: format!(
                "{}/protocol/openid-connect/token/introspect",
                config.issuer.trim_end_matches('/')
            ),
            config,
            client,
            cache: Mutex::new(BTreeMap::new()),
        })
    }

    async fn fetch(&self, token: &str) -> Result<Introspection, PolicyDenial> {
        let mut response = self
            .client
            .post(&self.endpoint)
            .basic_auth(
                encode_client_credential(&self.config.client_id),
                Some(encode_client_credential(self.config.client_secret.expose())),
            )
            .form(&[("token", token), ("token_type_hint", "access_token")])
            .send()
            .await
            .map_err(|_| PolicyDenial::Unavailable)?;
        if !response.status().is_success() {
            return Err(PolicyDenial::Unavailable);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| PolicyDenial::Unavailable)?
        {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(PolicyDenial::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body).map_err(|_| PolicyDenial::Unavailable)
    }

    async fn cached(&self, key: &[u8; 32]) -> Option<Introspection> {
        let mut cache = self.cache.lock().await;
        cache.retain(|_, entry| entry.expires_at > Instant::now());
        cache.get(key).map(|entry| entry.result.clone())
    }

    async fn remember(&self, key: [u8; 32], result: Introspection, lifetime: Duration) {
        if lifetime.is_zero() {
            return;
        }
        let mut cache = self.cache.lock().await;
        cache.retain(|_, entry| entry.expires_at > Instant::now());
        if cache.len() >= self.config.cache_capacity
            && let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, entry)| entry.expires_at)
                .map(|(key, _)| *key)
        {
            cache.remove(&oldest);
        }
        cache.insert(
            key,
            CacheEntry {
                expires_at: Instant::now() + lifetime,
                result,
            },
        );
    }

    async fn evaluate(
        &self,
        principal: &VerifiedPrincipal,
        token: &str,
    ) -> Result<Principal, PolicyDenial> {
        let hash = digest::digest(&digest::SHA256, token.as_bytes());
        let mut key = [0; 32];
        key.copy_from_slice(hash.as_ref());
        let cached = self.cached(&key).await;
        metrics::counter!("mcp_introspection_cache_requests_total", "outcome" => if cached.is_some() { "hit" } else { "miss" }).increment(1);
        let hit = cached.is_some();
        let result = match cached {
            Some(result) => result,
            None => self.fetch(token).await?,
        };
        if !result.active
            || result.sub.as_deref() != Some(principal.subject())
            || result
                .iss
                .as_deref()
                .is_some_and(|issuer| Some(issuer) != principal.issuer())
            || result
                .client_id
                .as_deref()
                .is_some_and(|client| Some(client) != principal.client_id())
        {
            return Err(PolicyDenial::Inactive);
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| PolicyDenial::Unavailable)?;
        let expires = Duration::from_secs(result.exp.ok_or(PolicyDenial::Unavailable)?);
        if expires <= now {
            return Err(PolicyDenial::Inactive);
        }
        let effective = Principal::from(principal.clone()).with_scopes(
            result
                .scope
                .as_deref()
                .unwrap_or_default()
                .split_ascii_whitespace()
                .map(str::to_owned),
        );
        if !hit {
            self.remember(key, result, self.config.cache_ttl.min(expires - now))
                .await;
        }
        Ok(effective)
    }
}

impl AuthenticationPolicy for KeycloakIntrospectionPolicy {
    fn authenticate<'a>(
        &'a self,
        principal: &'a VerifiedPrincipal,
        token: &'a str,
    ) -> PolicyFuture<'a> {
        Box::pin(async move {
            tokio::time::timeout(self.config.timeout, self.evaluate(principal, token))
                .await
                .unwrap_or(Err(PolicyDenial::Unavailable))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use baukit_auth::OidcVerifier;
    use baukit_test::MockOidcServer;
    use serde_json::{Value, json};
    use std::{
        error::Error,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use tokio::net::TcpListener;

    #[derive(Clone)]
    struct Endpoint {
        response: Arc<Mutex<(StatusCode, String)>>,
        calls: Arc<AtomicUsize>,
    }

    async fn endpoint(
        State(state): State<Endpoint>,
        headers: HeaderMap,
        axum::Form(form): axum::Form<BTreeMap<String, String>>,
    ) -> impl IntoResponse {
        assert_eq!(headers["authorization"], "Basic cmVzb3VyY2U6c2VjcmV0");
        assert_eq!(form["token_type_hint"], "access_token");
        assert!(form.contains_key("token"));
        state.calls.fetch_add(1, Ordering::SeqCst);
        state.response.lock().await.clone()
    }

    async fn principal() -> Result<VerifiedPrincipal, Box<dyn Error>> {
        let issuer = MockOidcServer::start().await?;
        let config = crate::McpConfig {
            issuer: issuer.issuer().into(),
            resource_url: "https://mcp.example/mcp".into(),
            ..Default::default()
        };
        let verifier = OidcVerifier::discover(
            baukit_auth::OidcConfig::new(&config.issuer, &config.resource_url)?
                .with_principal_claims(
                    baukit_auth::PrincipalClaimMapping::new().client_id_claim("azp"),
                ),
        )
        .await?;
        let token = issuer.mint(
            &issuer
                .claims("alice", &config.resource_url, Duration::from_secs(300))?
                .claim("scope", "read write")
                .claim("azp", "client"),
        )?;
        Ok(verifier.verify(&token).await?)
    }

    fn active() -> Value {
        json!({"active":true,"sub":"alice","client_id":"client","scope":"read injected", "exp":SystemTime::now().duration_since(UNIX_EPOCH).expect("clock").as_secs()+300})
    }

    #[tokio::test]
    async fn introspection_restricts_grants_and_rejects_invalid_responses()
    -> Result<(), Box<dyn Error>> {
        let state = Endpoint {
            response: Arc::new(Mutex::new((StatusCode::OK, active().to_string()))),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut config = KeycloakIntrospectionConfig::new(
            format!("http://{}", listener.local_addr()?),
            "resource",
            Secret::new("secret".to_owned()),
        );
        config.cache_ttl = Duration::ZERO;
        let policy = KeycloakIntrospectionPolicy::new(config)?;
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/protocol/openid-connect/token/introspect", post(endpoint))
                    .with_state(state.clone()),
            )
            .into_future(),
        );
        let principal = principal().await?;
        let result = policy.authenticate(&principal, "verified-token").await?;
        assert_eq!(
            result.scopes(),
            &std::collections::BTreeSet::from(["read".to_owned()])
        );
        assert_eq!(result.verified_identity(), &principal);
        for response in [
            json!({"active":false}),
            {
                let mut r = active();
                r["sub"] = json!("other");
                r
            },
            {
                let mut r = active();
                r["iss"] = json!("https://wrong.example");
                r
            },
            {
                let mut r = active();
                r["client_id"] = json!("other");
                r
            },
            {
                let mut r = active();
                r["exp"] = json!(1);
                r
            },
        ] {
            *state.response.lock().await = (StatusCode::OK, response.to_string());
            assert_eq!(
                policy.authenticate(&principal, "verified-token").await,
                Err(PolicyDenial::Inactive)
            );
        }
        let mut no_scope = active();
        no_scope.as_object_mut().ok_or("object")?.remove("scope");
        *state.response.lock().await = (StatusCode::OK, no_scope.to_string());
        assert!(
            policy
                .authenticate(&principal, "verified-token")
                .await?
                .scopes()
                .is_empty()
        );
        for response in [
            (StatusCode::UNAUTHORIZED, active().to_string()),
            (StatusCode::FOUND, active().to_string()),
            (StatusCode::OK, "invalid json".into()),
            (StatusCode::OK, "x".repeat(MAX_RESPONSE_BYTES + 1)),
            (
                StatusCode::OK,
                json!({"active":true,"sub":"alice"}).to_string(),
            ),
        ] {
            *state.response.lock().await = response;
            assert_eq!(
                policy.authenticate(&principal, "verified-token").await,
                Err(PolicyDenial::Unavailable)
            );
        }
        assert_eq!(state.calls.load(Ordering::SeqCst), 12);
        server.abort();
        assert!(server.await.expect_err("cancelled server").is_cancelled());
        Ok(())
    }

    #[tokio::test]
    async fn cached_active_result_expires_and_failures_are_not_cached() -> Result<(), Box<dyn Error>>
    {
        let state = Endpoint {
            response: Arc::new(Mutex::new((StatusCode::OK, active().to_string()))),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut config = KeycloakIntrospectionConfig::new(
            format!("http://{}", listener.local_addr()?),
            "resource",
            Secret::new("secret".to_owned()),
        );
        let ttl = Duration::from_secs(5);
        config.cache_ttl = ttl;
        let policy = KeycloakIntrospectionPolicy::new(config)?;
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new()
                    .route("/protocol/openid-connect/token/introspect", post(endpoint))
                    .with_state(state.clone()),
            )
            .into_future(),
        );
        let principal = principal().await?;
        let first = policy.authenticate(&principal, "first-token").await?;
        tokio::time::pause();
        *state.response.lock().await = (StatusCode::OK, json!({"active":false}).to_string());
        assert_eq!(policy.authenticate(&principal, "first-token").await?, first);
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);
        tokio::time::advance(ttl).await;
        tokio::time::resume();
        assert_eq!(
            policy.authenticate(&principal, "other-token").await,
            Err(PolicyDenial::Inactive)
        );
        assert_eq!(
            policy.authenticate(&principal, "first-token").await,
            Err(PolicyDenial::Inactive)
        );
        *state.response.lock().await = (StatusCode::SERVICE_UNAVAILABLE, String::new());
        assert_eq!(
            policy.authenticate(&principal, "first-token").await,
            Err(PolicyDenial::Unavailable)
        );
        *state.response.lock().await = (StatusCode::OK, active().to_string());
        assert_eq!(policy.authenticate(&principal, "first-token").await?, first);
        assert_eq!(state.calls.load(Ordering::SeqCst), 5);
        assert!(
            first
                .clone()
                .with_scopes(["write".to_owned()])
                .scopes()
                .is_empty()
        );
        server.abort();
        assert!(server.await.expect_err("cancelled server").is_cancelled());
        Ok(())
    }

    #[tokio::test(start_paused = true)]
    async fn cache_is_bounded_expires_without_sliding_and_can_be_disabled()
    -> Result<(), Box<dyn Error>> {
        let mut config = KeycloakIntrospectionConfig::new(
            "http://127.0.0.1",
            "resource",
            Secret::new("secret".to_owned()),
        );
        config.cache_capacity = 2;
        let policy = KeycloakIntrospectionPolicy::new(config)?;
        let result: Introspection = serde_json::from_value(active())?;
        policy
            .remember([1; 32], result.clone(), Duration::from_secs(1))
            .await;
        policy
            .remember([2; 32], result.clone(), Duration::from_secs(2))
            .await;
        assert!(policy.cached(&[1; 32]).await.is_some());
        policy
            .remember([3; 32], result.clone(), Duration::from_secs(3))
            .await;
        assert!(policy.cached(&[1; 32]).await.is_none());
        assert_eq!(policy.cache.lock().await.len(), 2);
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(policy.cached(&[2; 32]).await.is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(policy.cached(&[2; 32]).await.is_none());
        assert!(policy.cached(&[3; 32]).await.is_some());
        tokio::time::advance(Duration::from_secs(1)).await;
        assert!(policy.cached(&[3; 32]).await.is_none());
        policy.remember([4; 32], result, Duration::ZERO).await;
        assert!(policy.cache.lock().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn oauth_client_credentials_are_encoded_before_basic_auth() -> Result<(), Box<dyn Error>>
    {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let config = KeycloakIntrospectionConfig::new(
            format!("http://{}", listener.local_addr()?),
            "resource:client",
            Secret::new("secret +%".to_owned()),
        );
        let policy = KeycloakIntrospectionPolicy::new(config)?;
        let app = Router::new().route(
            "/protocol/openid-connect/token/introspect",
            post(|headers: HeaderMap| async move {
                if headers
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    != Some("Basic cmVzb3VyY2UlM0FjbGllbnQ6c2VjcmV0KyUyQiUyNQ==")
                {
                    return (StatusCode::UNAUTHORIZED, axum::Json(json!({})));
                }
                (StatusCode::OK, axum::Json(active()))
            }),
        );
        let server = tokio::spawn(axum::serve(listener, app).into_future());
        let result = policy
            .authenticate(&principal().await?, "verified-token")
            .await?;
        assert_eq!(
            result.scopes(),
            &std::collections::BTreeSet::from(["read".to_owned()])
        );
        server.abort();
        assert!(server.await.expect_err("cancelled server").is_cancelled());
        Ok(())
    }

    #[tokio::test]
    async fn timeout_fails_closed() -> Result<(), Box<dyn Error>> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let mut config = KeycloakIntrospectionConfig::new(
            format!("http://{}", listener.local_addr()?),
            "resource",
            Secret::new("secret".to_owned()),
        );
        config.timeout = Duration::from_millis(20);
        let policy = KeycloakIntrospectionPolicy::new(config)?;
        let server = tokio::spawn(
            axum::serve(
                listener,
                Router::new().route(
                    "/protocol/openid-connect/token/introspect",
                    post(|| async {
                        std::future::pending::<()>().await;
                        ""
                    }),
                ),
            )
            .into_future(),
        );
        assert_eq!(
            policy
                .authenticate(&VerifiedPrincipal::new("alice"), "verified-token")
                .await,
            Err(PolicyDenial::Unavailable)
        );
        server.abort();
        assert!(server.await.expect_err("cancelled server").is_cancelled());
        Ok(())
    }

    #[test]
    fn credentials_urls_and_limits_are_validated() {
        for (issuer, id, secret, timeout, ttl, capacity) in [
            ("http://remote.example", "resource", "secret", 2, 5, 1),
            ("https://identity.example", "", "secret", 2, 5, 1),
            ("https://identity.example", "resource", "", 2, 5, 1),
            ("https://identity.example", "resource", "secret", 0, 5, 1),
            ("https://identity.example", "resource", "secret", 31, 5, 1),
            ("https://identity.example", "resource", "secret", 2, 31, 1),
            ("https://identity.example", "resource", "secret", 2, 5, 0),
            (
                "https://identity.example",
                "resource",
                "secret",
                2,
                5,
                MAX_CACHE_CAPACITY + 1,
            ),
        ] {
            let mut config =
                KeycloakIntrospectionConfig::new(issuer, id, Secret::new(secret.to_owned()));
            config.timeout = Duration::from_secs(timeout);
            config.cache_ttl = Duration::from_secs(ttl);
            config.cache_capacity = capacity;
            assert!(KeycloakIntrospectionPolicy::new(config).is_err());
        }
    }
}
