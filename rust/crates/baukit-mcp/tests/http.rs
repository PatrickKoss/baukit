#![cfg(test)]
mod support;

use std::{
    error::Error,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use baukit_mcp::{
    Implementation, JwtOnlyPolicy, McpConfig, McpServices, Principal, ScopedTool, ToolAnnotations,
    ToolError, ToolFuture, ToolService, router,
};
use baukit_ratelimit::{
    InMemoryRateLimitStore, Quota, RateLimitDecision, RateLimitStore, RateLimitStoreError,
};
use baukit_test::{InMemoryApiTokenStore, MockOidcServer};
use serde_json::{Value, json};
use tower::ServiceExt;

const API_TOKEN_OWNER: &str = "00000000-0000-4000-8000-000000000001";

struct IdentityTool;
impl ToolService for IdentityTool {
    fn tools(&self) -> Vec<ScopedTool> {
        vec![ScopedTool {
            name: "identity".into(),
            description: "Read the verified identity".into(),
            input_schema: json!({"type":"object","properties":{},"additionalProperties":false}),
            output_schema: None,
            required_scopes: vec!["identity:read".into(), "account:read".into()],
            read_only: true,
            annotations: Default::default(),
        }]
    }
    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        name: &'a str,
        arguments: Value,
        _cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            if name != "identity" || arguments != json!({}) {
                return Err(ToolError::new(
                    "invalid_arguments",
                    "Empty arguments required",
                ));
            }
            Ok(
                json!({"subject":principal.subject(), "issuer":principal.issuer(), "client":principal.client_id(), "scopes":principal.scopes()}),
            )
        })
    }
}

fn config(issuer: &MockOidcServer) -> McpConfig {
    McpConfig {
        enabled: true,
        issuer: issuer.issuer().into(),
        resource_url: "https://mcp.example/mcp".into(),
        allowed_hosts: vec!["mcp.example".into()],
        ..Default::default()
    }
}

async fn verifier(
    config: &McpConfig,
) -> Result<Arc<dyn baukit_auth::IdentityVerifier>, Box<dyn Error>> {
    Ok(Arc::new(
        baukit_auth::OidcVerifier::discover(
            baukit_auth::OidcConfig::new(&config.issuer, &config.resource_url)?
                .with_clock_skew(Duration::ZERO)
                .with_principal_claims(
                    baukit_auth::PrincipalClaimMapping::new().client_id_claim("azp"),
                ),
        )
        .await?,
    ))
}

async fn app(config: McpConfig) -> Result<Router, Box<dyn Error>> {
    Ok(router(
        config.clone(),
        verifier(&config).await?,
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?)
}

async fn send(
    app: &Router,
    token: Option<&str>,
    message: Value,
) -> Result<(StatusCode, http::HeaderMap, Value), Box<dyn Error>> {
    send_with_origin(app, token, message, None).await
}

async fn send_with_origin(
    app: &Router,
    token: Option<&str>,
    mut message: Value,
    origin: Option<&str>,
) -> Result<(StatusCode, http::HeaderMap, Value), Box<dyn Error>> {
    if message["method"] != "initialize" {
        message["params"]["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientInfo":{"name":"protocol-test","version":"1"},"io.modelcontextprotocol/clientCapabilities":{}});
    }
    let version = if message["method"] == "initialize" {
        "2025-11-25"
    } else {
        "2026-07-28"
    };
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, "mcp.example")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", version)
        .header("mcp-method", message["method"].as_str().unwrap_or_default());
    if let Some(name) = message
        .pointer("/params/name")
        .or_else(|| message.pointer("/params/uri"))
        .and_then(Value::as_str)
    {
        builder = builder.header("mcp-name", name);
    }
    if let Some(origin) = origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(builder.body(Body::from(message.to_string()))?)
        .await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1024 * 1024).await?;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)?
    };
    Ok((status, headers, value))
}

fn list() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
}
fn call() -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"identity","arguments":{}}})
}

struct ApiTokenTools;

impl ToolService for ApiTokenTools {
    fn tools(&self) -> Vec<ScopedTool> {
        let mut tools = IdentityTool.tools();
        let mut write = tools[0].clone();
        write.name = "write".into();
        write.required_scopes = vec!["account:write".into()];
        tools.push(write);
        tools
    }

    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        name: &'a str,
        arguments: Value,
        cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        IdentityTool.call(principal, name, arguments, cancellation)
    }
}

struct NarrowApiTokenScopes;

impl baukit_mcp::AuthenticationPolicy for NarrowApiTokenScopes {
    fn authenticate<'a>(
        &'a self,
        principal: &'a baukit_mcp::VerifiedPrincipal,
        _token: &'a str,
    ) -> baukit_mcp::PolicyFuture<'a> {
        Box::pin(async move {
            let effective = Principal::from(principal.clone())
                .with_scopes(["identity:read".into(), "injected:write".into()])
                .with_scopes([
                    "identity:read".into(),
                    "account:read".into(),
                    "injected:write".into(),
                ]);
            assert_eq!(
                effective.scopes(),
                &std::collections::BTreeSet::from(["identity:read".into()])
            );
            assert_eq!(effective.verified_identity(), principal);
            Ok(effective)
        })
    }
}

fn assert_insufficient_scope(
    status: StatusCode,
    headers: &http::HeaderMap,
    body: &Value,
    challenge: &str,
) -> Result<(), Box<dyn Error>> {
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body, &json!({"error":"insufficient_scope"}));
    assert!(
        headers[header::WWW_AUTHENTICATE]
            .to_str()?
            .contains(challenge)
    );
    Ok(())
}

fn tool_names(body: &Value) -> Result<Vec<&str>, Box<dyn Error>> {
    Ok(body["result"]["tools"]
        .as_array()
        .ok_or("tools")?
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect())
}

async fn api_token_tools_app(
    config: &McpConfig,
    verifier: Arc<dyn baukit_auth::IdentityVerifier>,
    policy: Arc<dyn baukit_mcp::AuthenticationPolicy>,
) -> Result<Router, Box<dyn Error>> {
    Ok(router(
        config.clone(),
        verifier,
        McpServices::new(Arc::new(ApiTokenTools)),
        Arc::new(InMemoryRateLimitStore::default()),
        policy,
    )
    .await?)
}

async fn assert_api_token_policy(
    app: &Router,
    secret: &str,
    owner: &str,
    narrowed: bool,
) -> Result<(), Box<dyn Error>> {
    let (status, _, body) = send(app, Some(secret), list()).await?;
    assert_eq!(status, StatusCode::OK);
    let names = tool_names(&body)?;
    assert_eq!(names, if narrowed { vec![] } else { vec!["identity"] });

    let (status, headers, body) = send(app, Some(secret), call()).await?;
    if narrowed {
        assert_insufficient_scope(status, &headers, &body, "insufficient_scope")?;
    } else {
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["result"]["structuredContent"]["scopes"],
            json!(["account:read", "identity:read"])
        );
        assert_eq!(body["result"]["structuredContent"]["subject"], owner);
    }

    let mut write = call();
    write["params"]["name"] = json!("write");
    let (status, headers, body) = send(app, Some(secret), write).await?;
    assert_insufficient_scope(status, &headers, &body, "scope=\"account:write\"")
}

async fn assert_oidc_scopes_unaffected_by_api_tokens(
    app: &Router,
    issuer: &MockOidcServer,
    config: &McpConfig,
) -> Result<(), Box<dyn Error>> {
    let oidc = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "identity:read account:read"),
    )?;
    let (status, _, body) = send(app, Some(&oidc), call()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["structuredContent"]["subject"], "alice");
    assert_eq!(
        body["result"]["structuredContent"]["scopes"],
        json!(["account:read", "identity:read"])
    );
    Ok(())
}

#[tokio::test]
async fn api_token_grants_filter_tools_and_policies_can_only_narrow() -> Result<(), Box<dyn Error>>
{
    use baukit_auth::{ApiTokenService, ApiTokenVerifier, IdentityVerifier, NewApiToken};

    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let tokens = ApiTokenService::new(Arc::new(InMemoryApiTokenStore::new()));
    let issued = tokens
        .issue(
            API_TOKEN_OWNER.parse()?,
            NewApiToken::new("MCP").with_grants(["identity:read", "account:read"]),
        )
        .await?;
    let verifier = Arc::new(ApiTokenVerifier::new(
        tokens.clone(),
        verifier(&config).await?,
    ));
    let verified = verifier.verify(&issued.secret).await?;
    assert!(verified.scopes().is_empty());
    assert_eq!(verified.grants(), Some(&issued.token.grants));
    assert_eq!(
        Principal::from(verified.clone()).scopes(),
        &issued.token.grants
    );

    let owner = issued.token.owner_id.to_string();
    for (policy, narrowed) in [
        (
            Arc::new(JwtOnlyPolicy) as Arc<dyn baukit_mcp::AuthenticationPolicy>,
            false,
        ),
        (Arc::new(NarrowApiTokenScopes), true),
    ] {
        let app = api_token_tools_app(&config, verifier.clone(), policy).await?;
        assert_api_token_policy(&app, &issued.secret, &owner, narrowed).await?;
    }

    let no_grants = tokens
        .issue(API_TOKEN_OWNER.parse()?, NewApiToken::new("No grants"))
        .await?;
    let app = api_token_tools_app(&config, verifier, Arc::new(JwtOnlyPolicy)).await?;
    let (status, _, body) = send(&app, Some(&no_grants.secret), list()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["result"]["tools"], json!([]));
    assert_eq!(
        send(&app, Some(&no_grants.secret), call()).await?.0,
        StatusCode::FORBIDDEN
    );
    assert_oidc_scopes_unaffected_by_api_tokens(&app, &issuer, &config).await
}

#[tokio::test]
async fn api_tokens_are_rechecked_for_revocation_and_expiry_on_every_request()
-> Result<(), Box<dyn Error>> {
    use baukit_auth::{
        ApiTokenRecord, ApiTokenService, ApiTokenStore, ApiTokenVerifier, NewApiToken,
        hash_api_token,
    };

    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    for revoked in [true, false] {
        let store = Arc::new(InMemoryApiTokenStore::new());
        let tokens = ApiTokenService::new(store.clone());
        let issued = tokens
            .issue(
                API_TOKEN_OWNER.parse()?,
                NewApiToken::new("MCP").with_grants(["identity:read", "account:read"]),
            )
            .await?;
        let app = router(
            config.clone(),
            Arc::new(ApiTokenVerifier::new(
                tokens.clone(),
                verifier(&config).await?,
            )),
            McpServices::new(Arc::new(IdentityTool)),
            Arc::new(InMemoryRateLimitStore::default()),
            Arc::new(JwtOnlyPolicy),
        )
        .await?;
        for request in [list(), call()] {
            assert_eq!(
                send(&app, Some(&issued.secret), request).await?.0,
                StatusCode::OK
            );
        }
        if revoked {
            tokens
                .revoke(issued.token.owner_id, issued.token.id)
                .await?;
        } else {
            store
                .create(ApiTokenRecord {
                    id: issued.token.id,
                    owner_id: issued.token.owner_id,
                    name: issued.token.name.clone(),
                    secret_hash: hash_api_token(&issued.secret),
                    display_prefix: issued.token.display_prefix.clone(),
                    grants: issued.token.grants.clone(),
                    created_at: issued.token.created_at,
                    expires_at: Some(SystemTime::now().into()),
                })
                .await?;
        }
        for request in [list(), call()] {
            let (status, headers, body) = send(&app, Some(&issued.secret), request).await?;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert_eq!(body, json!({"error":"invalid_token"}));
            assert!(
                headers[header::WWW_AUTHENTICATE]
                    .to_str()?
                    .contains("invalid_token")
            );
        }
    }
    Ok(())
}

#[tokio::test]
async fn keycloak_introspection_rejects_api_tokens_without_contacting_provider()
-> Result<(), Box<dyn Error>> {
    use baukit_auth::{ApiTokenService, ApiTokenVerifier, NewApiToken};
    use baukit_mcp::{KeycloakIntrospectionConfig, KeycloakIntrospectionPolicy};

    // A connection queued on this listener would expose a PAT to the provider.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let tokens = ApiTokenService::new(Arc::new(InMemoryApiTokenStore::new()));
    let issued = tokens
        .issue(
            API_TOKEN_OWNER.parse()?,
            NewApiToken::new("MCP").with_grants(["identity:read", "account:read"]),
        )
        .await?;
    let policy = KeycloakIntrospectionPolicy::new(KeycloakIntrospectionConfig::new(
        format!("http://{}", listener.local_addr()?),
        "resource",
        baukit_config::Secret::new("secret".into()),
    ))?;
    let app = router(
        config.clone(),
        Arc::new(ApiTokenVerifier::new(tokens, verifier(&config).await?)),
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(policy),
    )
    .await?;
    for request in [list(), call()] {
        let (status, _, body) = send(&app, Some(&issued.secret), request).await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body, json!({"error":"invalid_token"}));
    }
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn api_tokens_work_alongside_oidc_client_and_audience_restrictions()
-> Result<(), Box<dyn Error>> {
    use baukit_auth::{ApiTokenService, ApiTokenVerifier, NewApiToken, OidcConfig, OidcVerifier};

    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let tokens = ApiTokenService::new(Arc::new(InMemoryApiTokenStore::new()));
    let issued = tokens
        .issue(
            API_TOKEN_OWNER.parse()?,
            NewApiToken::new("MCP").with_grants(["identity:read", "account:read"]),
        )
        .await?;
    let oidc = OidcVerifier::discover(
        OidcConfig::new(&config.issuer, &config.resource_url)?
            .with_allowed_clients(["mcp-client"])?
            .with_principal_claims(
                baukit_auth::PrincipalClaimMapping::new().client_id_claim("azp"),
            ),
    )
    .await?;
    let app = router(
        config.clone(),
        Arc::new(ApiTokenVerifier::new(tokens, Arc::new(oidc))),
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    let (status, _, body) = send(&app, Some(&issued.secret), call()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["result"]["structuredContent"],
        json!({"subject":API_TOKEN_OWNER,"issuer":null,"client":null,
            "scopes":["account:read","identity:read"]})
    );
    for (audience, client, allowed) in [
        (config.resource_url.as_str(), Some("mcp-client"), true),
        (config.resource_url.as_str(), Some("other-client"), false),
        (config.resource_url.as_str(), None, false),
        ("product-rest", Some("mcp-client"), false),
    ] {
        let mut claims = issuer
            .claims("alice", audience, Duration::from_secs(300))?
            .claim("scope", "identity:read account:read");
        if let Some(client) = client {
            claims = claims.claim("azp", client);
        }
        let token = issuer.mint(&claims)?;
        for request in [list(), call()] {
            let (status, _, body) = send(&app, Some(&token), request).await?;
            assert_eq!(
                status,
                if allowed {
                    StatusCode::OK
                } else {
                    StatusCode::UNAUTHORIZED
                }
            );
            if !allowed {
                assert_eq!(body, json!({"error":"invalid_token"}));
            } else if body["result"].get("structuredContent").is_some() {
                assert_eq!(body["result"]["structuredContent"]["client"], "mcp-client");
                assert_eq!(
                    body["result"]["structuredContent"]["scopes"],
                    json!(["account:read", "identity:read"])
                );
            }
        }
    }
    Ok(())
}

#[tokio::test]
async fn api_token_store_failures_return_unavailable_without_a_login_challenge()
-> Result<(), Box<dyn Error>> {
    use baukit_auth::{ApiTokenStoreError, ApiTokenVerifier, NewApiToken};

    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let (store, tokens) = InMemoryApiTokenStore::with_service();
    let issued = tokens
        .issue(
            API_TOKEN_OWNER.parse()?,
            NewApiToken::new("MCP").with_grants(["identity:read", "account:read"]),
        )
        .await?;
    let app = router(
        config.clone(),
        Arc::new(ApiTokenVerifier::new(tokens, verifier(&config).await?)),
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    assert_eq!(
        send(&app, Some(&issued.secret), call()).await?.0,
        StatusCode::OK
    );
    store.fail_with(ApiTokenStoreError::internal(format!(
        "private store diagnostic {}",
        issued.secret
    )));
    for request in [list(), call()] {
        let (status, headers, body) = send(&app, Some(&issued.secret), request).await?;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body, json!({"error":"identity_unavailable"}));
        assert_eq!(headers[header::RETRY_AFTER], "5");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(!headers.contains_key(header::WWW_AUTHENTICATE));
    }
    Ok(())
}

#[tokio::test]
async fn metadata_and_oauth_errors_follow_the_resource_contract() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let app = app(config.clone()).await?;
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header(header::HOST, "mcp.example")
                    .body(Body::empty())?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let document: Value = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
        assert_eq!(
            document,
            json!({"resource":config.resource_url,"authorization_servers":[issuer.issuer()],"bearer_methods_supported":["header"],"scopes_supported":["account:read","identity:read"]})
        );
    }
    let (status, headers, body) = send(&app, None, list()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body, json!({"error":"invalid_token"}));
    assert_eq!(
        headers[header::WWW_AUTHENTICATE],
        "Bearer resource_metadata=\"https://mcp.example/.well-known/oauth-protected-resource\", error=\"invalid_token\", scope=\"account:read identity:read\""
    );
    let claims = issuer.claims("alice", &config.resource_url, Duration::from_secs(300))?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    for claims in [
        claims.clone().audience("https://other.example/mcp"),
        claims.clone().expires_at(now - 1),
        claims.clone().issuer("https://wrong.example"),
        claims.clone().not_before(now + 300),
    ] {
        let (status, headers, body) = send(&app, Some(&issuer.mint(&claims)?), list()).await?;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert!(
            headers[header::WWW_AUTHENTICATE]
                .to_str()?
                .contains("resource_metadata=")
        );
        assert_eq!(body, json!({"error":"invalid_token"}));
    }
    for scopes in ["", "identity:read", "identity:read account:reader"] {
        let token = issuer.mint(&claims.clone().claim("scope", scopes))?;
        let (status, _, body) = send(&app, Some(&token), list()).await?;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["result"]["tools"], json!([]));
        let (status, headers, body) = send(&app, Some(&token), call()).await?;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body, json!({"error":"insufficient_scope"}));
        assert!(
            headers[header::WWW_AUTHENTICATE]
                .to_str()?
                .contains("scope=\"identity:read account:read\"")
        );
    }
    Ok(())
}

#[tokio::test]
async fn initialize_list_and_call_preserve_verified_principal() -> Result<(), Box<dyn Error>> {
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "identity:read account:read")
            .claim("azp", "registered-client"),
    )?;
    let app = app(config).await?;
    let (status, _, result) = send(&app, Some(&token), json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"protocol-test","version":"1"}}})).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        result["result"]["serverInfo"],
        json!({
            "name": "baukit-mcp", "version": env!("CARGO_PKG_VERSION")
        })
    );
    assert!(result["result"].get("instructions").is_none());
    assert!(result["result"]["capabilities"].get("resources").is_none());
    assert!(result["result"]["capabilities"].get("prompts").is_none());
    let (status, _, result) = send(&app, Some(&token), list()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["result"]["tools"][0]["name"], "identity");
    assert_eq!(
        result["result"]["tools"][0]["annotations"]["readOnlyHint"],
        true
    );
    let (status, headers, result) = send(&app, Some(&token), call()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        result["result"]["structuredContent"],
        json!({"subject":"alice", "issuer":issuer.issuer(), "client":"registered-client", "scopes":["account:read","identity:read"]})
    );
    assert_eq!(
        result["result"]["content"],
        json!([{
            "type": "text", "text": result["result"]["structuredContent"].to_string()
        }])
    );
    let (status, _, body) = send(&app, None, call()).await?;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "invalid_token");
    Ok(())
}

#[tokio::test]
async fn body_limit_rate_limit_and_authority_rejection_are_enforced() -> Result<(), Box<dyn Error>>
{
    let issuer = MockOidcServer::start().await?;
    let mut config = config(&issuer);
    config.max_request_body_bytes = 256;
    config.requests_per_minute = 1;
    let token =
        issuer.mint(&issuer.claims("alice", &config.resource_url, Duration::from_secs(300))?)?;
    let app = app(config).await?;
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/.well-known/oauth-protected-resource")
                .header(header::HOST, "evil.example")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let (status, _, body) = send(
        &app,
        Some(&token),
        json!({"method":"tools/list","padding":"x".repeat(300)}),
    )
    .await?;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(body["error"], "request_too_large");
    let (status, headers, body) = send(&app, Some(&token), list()).await?;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(headers[header::RETRY_AFTER].to_str()?.parse::<u64>()? > 0);
    assert_eq!(body["error"], "rate_limited");
    Ok(())
}

struct UnavailableStore;

impl RateLimitStore for UnavailableStore {
    fn check_and_consume<'a>(
        &'a self,
        _key: &'a str,
        _quota: Quota,
    ) -> Pin<Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>>
    {
        Box::pin(async { Err(RateLimitStoreError::unavailable("private storage details")) })
    }
}

#[tokio::test]
async fn disabled_resources_are_absent_and_store_failure_fails_closed() -> Result<(), Box<dyn Error>>
{
    let disabled_issuer = MockOidcServer::start().await?;
    let disabled = router(
        McpConfig::default(),
        verifier(&config(&disabled_issuer)).await?,
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(UnavailableStore),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    for path in ["/mcp", "/.well-known/oauth-protected-resource"] {
        let response = disabled
            .clone()
            .oneshot(Request::builder().uri(path).body(Body::empty())?)
            .await?;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token =
        issuer.mint(&issuer.claims("alice", &config.resource_url, Duration::from_secs(300))?)?;
    let app = router(
        config.clone(),
        verifier(&config).await?,
        McpServices::new(Arc::new(IdentityTool)),
        Arc::new(UnavailableStore),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    let (status, headers, body) = send(&app, Some(&token), list()).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body, json!({"error": "rate_limit_unavailable"}));
    Ok(())
}

struct ProductPolicy {
    denial: Option<baukit_mcp::PolicyDenial>,
    scopes: Vec<String>,
    calls: std::sync::atomic::AtomicUsize,
}

impl baukit_mcp::AuthenticationPolicy for ProductPolicy {
    fn authenticate<'a>(
        &'a self,
        principal: &'a baukit_mcp::VerifiedPrincipal,
        token: &'a str,
    ) -> baukit_mcp::PolicyFuture<'a> {
        Box::pin(async move {
            assert!(!token.is_empty());
            assert_eq!(principal.subject(), "alice");
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if let Some(denial) = &self.denial {
                return Err(denial.clone());
            }
            Ok(Principal::from(principal.clone())
                .with_subject("linked-account")
                .with_scopes(self.scopes.clone()))
        })
    }
}

struct ConflictTool;

fn conflict_schema() -> Value {
    json!({"type":"object","properties":{"data":{"type":"null"},"error":{"type":"object","properties":{"code":{"const":"stale_revision"},"message":{"type":"string"},"status":{"type":"integer","minimum":400,"maximum":599},"details":{"type":"object","properties":{"currentRevision":{"type":"integer","minimum":0}},"required":["currentRevision"],"additionalProperties":false}},"required":["code","message","status","details"],"additionalProperties":false}},"required":["data","error"],"additionalProperties":false})
}

impl ToolService for ConflictTool {
    fn tools(&self) -> Vec<ScopedTool> {
        vec![ScopedTool {
            name: "save".into(),
            description: "Save a revision".into(),
            input_schema: json!({"type":"object"}),
            output_schema: Some(conflict_schema()),
            required_scopes: vec!["items:write".into()],
            read_only: false,
            annotations: Default::default(),
        }]
    }
    fn call<'a>(
        &'a self,
        _principal: &'a Principal,
        _name: &'a str,
        _arguments: Value,
        _cancellation: baukit_mcp::CancellationToken,
    ) -> ToolFuture<'a> {
        Box::pin(async {
            Err(ToolError::new("stale_revision", "Reload revision 7 before saving").with_structured_content(json!({"data":null,"error":{"code":"stale_revision","message":"Reload revision 7 before saving","status":409,"details":{"currentRevision":7}}})))
        })
    }
}

fn request(method: &str, params: &Value) -> Value {
    json!({"jsonrpc":"2.0","id":42,"method":method,"params":params})
}

#[tokio::test(flavor = "current_thread")]
async fn scope_denials_record_the_http_status_before_dispatch() -> Result<(), Box<dyn Error>> {
    use metrics_util::debugging::{DebugValue, DebuggingRecorder};

    let issuer = MockOidcServer::start().await?;
    let config = config(&issuer);
    let token = issuer.mint(
        &issuer
            .claims("alice", &config.resource_url, Duration::from_secs(300))?
            .claim("scope", "unrelated:read"),
    )?;
    let catalog = Arc::new(support::Catalog::default());
    let prompts = Arc::new(support::Recommendations::default());
    let services = McpServices::new(Arc::new(IdentityTool))
        .with_resources(catalog.clone())
        .with_prompts(prompts.clone());
    let app = router(
        config.clone(),
        verifier(&config).await?,
        services,
        Arc::new(InMemoryRateLimitStore::default()),
        Arc::new(JwtOnlyPolicy),
    )
    .await?;
    for (method, params, scope) in [
        (
            "tools/call",
            json!({"name":"identity","arguments":{}}),
            "identity:read account:read",
        ),
        (
            "resources/read",
            json!({"uri":"product://items/42"}),
            "items:read",
        ),
        (
            "prompts/get",
            json!({"name":"next-steps","arguments":{"language":"de"}}),
            "learning:read",
        ),
    ] {
        let recorder = DebuggingRecorder::new();
        let snapshotter = recorder.snapshotter();
        let _guard = metrics::set_default_local_recorder(&recorder);
        let (status, headers, body) = send(&app, Some(&token), request(method, &params)).await?;
        assert_eq!(status, StatusCode::FORBIDDEN, "{method}");
        assert_eq!(body, json!({"error":"insufficient_scope"}));
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            headers[header::WWW_AUTHENTICATE],
            format!(
                "Bearer resource_metadata=\"https://mcp.example/.well-known/oauth-protected-resource\", error=\"insufficient_scope\", scope=\"{scope}\""
            )
        );
        let measurements = snapshotter.snapshot().into_vec();
        let requests = measurements
            .iter()
            .filter(|(key, _, _, _)| key.key().name() == "mcp_requests_total")
            .collect::<Vec<_>>();
        assert_eq!(requests.len(), 1, "{method}");
        let (key, _, _, value) = requests[0];
        assert_eq!(
            key.key()
                .labels()
                .map(|label| (label.key(), label.value()))
                .collect::<Vec<_>>(),
            [("status", "403")],
            "{method}"
        );
        assert_eq!(*value, DebugValue::Counter(1), "{method}");
    }
    assert!(catalog.0.lock().expect("reads").is_empty());
    assert!(prompts.0.lock().expect("prompts").is_empty());
    Ok(())
}

#[path = "http/cancellation.rs"]
mod cancellation;
#[path = "http/capabilities.rs"]
mod capabilities;
#[path = "http/metadata.rs"]
mod metadata;
#[path = "http/policy.rs"]
mod policy;

#[cfg(feature = "suite")]
#[path = "support/suite.rs"]
mod suite;
