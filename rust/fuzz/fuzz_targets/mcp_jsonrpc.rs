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

/// Set while a request that must never reach a tool is in flight.
static TOOL_FORBIDDEN: AtomicBool = AtomicBool::new(false);
const HOSTS: [Option<&str>; 4] = [
    Some(HOST),
    Some("evil.example"),
    None,
    Some("mcp.example:444"),
];
const ORIGINS: [Option<&str>; 4] = [
    None,
    Some("https://mcp.example"),
    Some("https://evil.example"),
    Some("null"),
];
const PROTOCOLS: [Option<&str>; 4] = [
    Some("2026-07-28"),
    None,
    Some("1999-01-01"),
    Some("2026-07-28\n"),
];

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
            !TOOL_FORBIDDEN.load(Ordering::SeqCst),
            "a request without host, origin or scope rights reached a tool"
        );
        Box::pin(async { Ok(json!({})) })
    }
}

struct Harness {
    runtime: Runtime,
    app: Router,
    token: String,
    unscoped_token: String,
    _server: MockOidcServer,
}

fn harness() -> &'static Harness {
    static HARNESS: OnceLock<Harness> = OnceLock::new();
    HARNESS.get_or_init(|| {
        let runtime = Runtime::new().expect("tokio runtime");
        let (server, app, token, unscoped_token) = runtime.block_on(async {
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
            let unscoped = server
                .claims("fuzz-subject", RESOURCE_URL, TOKEN_LIFETIME)
                .expect("claims")
                .claim("scope", "other:read");
            let unscoped_token = server.mint(&unscoped).expect("token");
            (server, app, token, unscoped_token)
        });
        Harness {
            runtime,
            app,
            token,
            unscoped_token,
            _server: server,
        }
    })
}

#[derive(Clone, Copy)]
struct Variant {
    host: Option<&'static str>,
    origin: Option<&'static str>,
    protocol: Option<&'static str>,
    token: u8,
}

impl Variant {
    fn from_selector(selector: u8) -> Self {
        let pick = |shift: u8| usize::from((selector >> shift) & 0b11);
        Self {
            host: HOSTS[pick(0)],
            origin: ORIGINS[pick(2)],
            protocol: PROTOCOLS[pick(4)],
            token: (selector >> 6) % 3,
        }
    }

    fn trusted_transport(self) -> bool {
        self.host == Some(HOST) && matches!(self.origin, None | Some("https://mcp.example"))
    }
}

fn request(body: &[u8], variant: Variant, token: Option<&str>) -> Option<Request<Body>> {
    let mut builder = Request::builder()
        .method("POST")
        .uri("/mcp")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCEPT, "application/json, text/event-stream");
    if let Some(host) = variant.host {
        builder = builder.header(header::HOST, host);
    }
    if let Some(origin) = variant.origin {
        builder = builder.header(header::ORIGIN, origin);
    }
    if let Some(protocol) = variant.protocol {
        builder = builder.header("mcp-protocol-version", protocol);
    }
    if let Some(method) =
        json_field(body, "method").and_then(|value| value.as_str().map(str::to_owned))
    {
        builder = builder.header("mcp-method", method);
    }
    if let Some(token) = token {
        builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    builder.body(Body::from(body.to_vec())).ok()
}

fn json_field(body: &[u8], name: &str) -> Option<Value> {
    serde_json::from_slice::<Value>(body)
        .ok()?
        .get(name)
        .cloned()
}

fn calls_probe(body: &[u8]) -> bool {
    json_field(body, "method").as_ref().and_then(Value::as_str) == Some("tools/call")
}

async fn send(request: Request<Body>, tool_forbidden: bool) -> (StatusCode, Vec<u8>) {
    TOOL_FORBIDDEN.store(tool_forbidden, Ordering::SeqCst);
    let response = harness()
        .app
        .clone()
        .oneshot(request)
        .await
        .expect("infallible");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), MAX_RESPONSE_BYTES)
        .await
        .expect("response body");
    TOOL_FORBIDDEN.store(false, Ordering::SeqCst);
    (status, bytes.to_vec())
}

fn check_success(body: &[u8], status: StatusCode, response: &[u8]) {
    if !status.is_success() {
        return;
    }
    match json_field(body, "id") {
        None if json_field(body, "method").is_some() => {
            assert_eq!(status, StatusCode::ACCEPTED, "notifications get 202");
        }
        Some(id) => {
            if let Ok(reply) = serde_json::from_slice::<Value>(response) {
                assert_eq!(
                    reply.get("id"),
                    Some(&id),
                    "response id must echo the request id"
                );
            }
        }
        None => {}
    }
}

fuzz_target!(|data: &[u8]| {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    let variant = Variant::from_selector(selector);
    let harness = harness();
    harness.runtime.block_on(async {
        if let Some(anonymous) = request(body, variant, None) {
            let (status, _) = send(anonymous, true).await;
            assert!(!status.is_success(), "anonymous request succeeded");
        }
        let (token, scoped) = match variant.token {
            0 => (harness.token.as_str(), true),
            1 => (harness.unscoped_token.as_str(), false),
            _ => ("not-a-jwt", false),
        };
        let Some(authenticated) = request(body, variant, Some(token)) else {
            return;
        };
        let tool_forbidden = !variant.trusted_transport() || (!scoped && calls_probe(body));
        let (status, response) = send(authenticated, tool_forbidden).await;
        if !variant.trusted_transport() {
            assert!(!status.is_success(), "untrusted host or origin succeeded");
        }
        check_success(body, status, &response);
    });
});
