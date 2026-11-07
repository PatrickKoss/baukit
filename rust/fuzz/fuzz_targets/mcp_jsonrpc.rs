#![no_main]

use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{Router, body::Body};
use baukit_auth::{OidcConfig, OidcVerifier};
use baukit_mcp::{
    JwtOnlyPolicy, McpConfig, McpServices, Principal, ScopedTool, ToolFuture, ToolService, router,
};
use baukit_ratelimit::InMemoryRateLimitStore;
use baukit_test::MockOidcServer;
use http::{Request, StatusCode, header};
use libfuzzer_sys::fuzz_target;
use serde_json::{Value, json};
use tokio::runtime::Runtime;
use tower::ServiceExt;

const RESOURCE_URL: &str = "https://mcp.example/mcp";
const HOST: &str = "mcp.example";
const REQUESTS_PER_MINUTE: u64 = 1_000_000_000;
const TOKEN_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
const PROBE_SCOPE: &str = "probe:read";
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

static UNAUTHENTICATED_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

struct ProbeTool;

impl ToolService for ProbeTool {
    fn tools(&self) -> Vec<ScopedTool> {
        vec![ScopedTool {
            name: "probe".into(),
            description: "Records that a call reached a tool".into(),
            input_schema: json!({"type":"object"}),
            output_schema: None,
            required_scopes: vec![PROBE_SCOPE.into()],
            read_only: true,
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
        assert!(
            !UNAUTHENTICATED_IN_FLIGHT.load(Ordering::SeqCst),
            "an unauthenticated request reached a tool"
        );
        Box::pin(async { Ok(json!({})) })
    }
}

struct Harness {
    runtime: Runtime,
    app: Router,
    token: String,
    _server: MockOidcServer,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let runtime = Runtime::new().expect("tokio runtime");
        let (server, app, token) = runtime.block_on(async {
            let server = MockOidcServer::start().await.expect("mock issuer");
            let config = McpConfig {
                enabled: true,
                issuer: server.issuer().into(),
                resource_url: RESOURCE_URL.into(),
                allowed_hosts: vec![HOST.into()],
                requests_per_minute: REQUESTS_PER_MINUTE,
                ..Default::default()
            };
            let oidc = OidcConfig::new(server.issuer(), RESOURCE_URL).expect("oidc config");
            let verifier = OidcVerifier::from_jwks_uri(oidc, server.jwks_url()).expect("verifier");
            let app = router(
                config,
                Arc::new(verifier),
                McpServices::new(Arc::new(ProbeTool)),
                Arc::new(InMemoryRateLimitStore::default()),
                Arc::new(JwtOnlyPolicy),
            )
            .await
            .expect("mcp router");
            let claims = server
                .claims("fuzz-subject", RESOURCE_URL, TOKEN_LIFETIME)
                .expect("claims")
                .claim("scope", PROBE_SCOPE);
            let token = server.mint(&claims).expect("token");
            (server, app, token)
        });
        Harness {
            runtime,
            app,
            token,
            _server: server,
        }
    })
}

fn request(body: &[u8], token: Option<&str>) -> Option<Request<Body>> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::HOST, HOST)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("mcp-protocol-version", "2026-07-28");
    if let Some(method) = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
    {
        builder = builder.header("mcp-method", method);
    }
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_vec())).ok()
}

fuzz_target!(|data: &[u8]| {
    let harness = harness();
    harness.runtime.block_on(async {
        if let Some(anonymous) = request(data, None) {
            UNAUTHENTICATED_IN_FLIGHT.store(true, Ordering::SeqCst);
            let response = harness
                .app
                .clone()
                .oneshot(anonymous)
                .await
                .expect("infallible");
            UNAUTHENTICATED_IN_FLIGHT.store(false, Ordering::SeqCst);
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        if let Some(authenticated) = request(data, Some(&harness.token)) {
            let response = harness
                .app
                .clone()
                .oneshot(authenticated)
                .await
                .expect("infallible");
            axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
                .await
                .expect("response body");
        }
    });
});
