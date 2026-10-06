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
use baukit_mcp::{McpConfig, Principal, ScopedTool, ToolError, ToolFuture, ToolService, router};
use baukit_ratelimit::{
    InMemoryRateLimitStore, Quota, RateLimitDecision, RateLimitStore, RateLimitStoreError,
};
use baukit_test::MockOidcServer;
use serde_json::{Value, json};
use tower::ServiceExt;

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
        }]
    }
    fn call<'a>(
        &'a self,
        principal: &'a Principal,
        name: &'a str,
        arguments: Value,
    ) -> ToolFuture<'a> {
        Box::pin(async move {
            if name != "identity" || arguments != json!({}) {
                return Err(ToolError {
                    code: "invalid_arguments".into(),
                    message: "Empty arguments required".into(),
                });
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

async fn app(config: McpConfig) -> Result<Router, Box<dyn Error>> {
    Ok(router(
        config,
        Arc::new(IdentityTool),
        Arc::new(InMemoryRateLimitStore::default()),
    )
    .await?)
}

async fn send(
    app: &Router,
    token: Option<&str>,
    mut message: Value,
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
    if let Some(name) = message.pointer("/params/name").and_then(Value::as_str) {
        builder = builder.header("mcp-name", name);
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
    Ok((status, headers, serde_json::from_slice(&body)?))
}

fn list() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})
}
fn call() -> Value {
    json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"identity","arguments":{}}})
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
    let disabled = router(
        McpConfig::default(),
        Arc::new(IdentityTool),
        Arc::new(UnavailableStore),
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
    let app = router(config, Arc::new(IdentityTool), Arc::new(UnavailableStore)).await?;
    let (status, headers, body) = send(&app, Some(&token), list()).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body, json!({"error": "rate_limit_unavailable"}));
    Ok(())
}
