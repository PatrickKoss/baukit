use std::{sync::Arc, time::Instant};

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
};
use baukit_auth::IdentityVerifier;
use baukit_ratelimit::{Quota, RateLimitStore};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::never::NeverSessionManager,
};
use serde_json::{Value, json};

use crate::{
    AuthenticationPolicy, McpConfig, McpConfigError, McpServices, PolicyDenial, Principal,
    VerifiedPrincipal,
    server::{ProductServer, RegisteredServices, permitted_scopes, valid_scope},
};

const MAX_BEARER_TOKEN_BYTES: usize = 16 * 1024;
const IDENTITY_UNAVAILABLE_RETRY_AFTER_SECONDS: u64 = 5;

#[derive(Clone)]
struct Security {
    config: McpConfig,
    verifier: Arc<dyn IdentityVerifier>,
    services: Arc<RegisteredServices>,
    store: Arc<dyn RateLimitStore>,
    quota: Quota,
    metadata_url: String,
    policy: Arc<dyn AuthenticationPolicy>,
}

/// Mounts `/mcp` and both RFC 9728 discovery paths using the supplied verifier.
///
/// For OIDC tokens, the caller must enforce a resource audience or the documented
/// provider equivalent. API-token verifiers must check the current token store
/// for revocation and expiry on every request. `config.issuer` names the OAuth
/// authorization server in discovery.
/// The rate-limit store may be shared Redis.
pub async fn router(
    config: McpConfig,
    verifier: Arc<dyn IdentityVerifier>,
    services: McpServices,
    store: Arc<dyn RateLimitStore>,
    policy: Arc<dyn AuthenticationPolicy>,
) -> Result<Router, McpConfigError> {
    if !config.enabled {
        return Ok(Router::new());
    }
    config.validate()?;
    let services = Arc::new(RegisteredServices::new(services)?);
    let metadata = config.metadata(services.scopes());
    let mut transport = StreamableHttpServerConfig::default();
    transport.legacy_session_mode = false;
    transport.json_response = true;
    transport.stateless_protocol_metadata_required = false;
    transport.max_request_body_bytes = config.max_request_body_bytes;
    transport.allowed_hosts = config.allowed_hosts.clone();
    let server_services = services.clone();
    let service = StreamableHttpService::new(
        move || Ok(ProductServer(server_services.clone())),
        Arc::new(NeverSessionManager::default()),
        transport,
    );
    let security = Security {
        quota: config.quota()?,
        metadata_url: config.metadata_url()?,
        config,
        verifier,
        services,
        store,
        policy,
    };
    let mcp = Router::new()
        .route_service("/mcp", service)
        .layer(middleware::from_fn_with_state(security.clone(), authorize));
    let discovery = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get({
                let metadata = metadata.clone();
                move || async move { Json(metadata) }
            }),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            get(move || async move { Json(metadata) }),
        )
        .layer(middleware::from_fn_with_state(security, check_authority));
    Ok(mcp.merge(discovery))
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&HeaderValue> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

fn allowed(headers: &HeaderMap, config: &McpConfig) -> bool {
    let host_allowed = single_header(headers, header::HOST)
        .and_then(|host| host.to_str().ok())
        .is_some_and(|host| {
            host.parse::<http::uri::Authority>().is_ok()
                && config
                    .allowed_hosts
                    .iter()
                    .any(|allowed| host.eq_ignore_ascii_case(allowed))
        });
    if !host_allowed {
        return false;
    }
    if headers.get(header::ORIGIN).is_none() {
        return true;
    }
    single_header(headers, header::ORIGIN)
        .and_then(|origin| origin.to_str().ok())
        .is_some_and(|origin| {
            config
                .allowed_origins
                .iter()
                .any(|allowed| origin == allowed)
        })
}

async fn check_authority(State(state): State<Security>, request: Request, next: Next) -> Response {
    if !allowed(request.headers(), &state.config) {
        return error(StatusCode::FORBIDDEN, "invalid_host_or_origin");
    }
    next.run(request).await
}

async fn authorize(State(state): State<Security>, request: Request, next: Next) -> Response {
    let started = Instant::now();
    let response = authorize_inner(&state, request, next).await;
    metrics::counter!("mcp_requests_total", "status" => response.status().as_u16().to_string())
        .increment(1);
    metrics::histogram!("mcp_request_duration_seconds").record(started.elapsed().as_secs_f64());
    response
}

async fn authorize_inner(state: &Security, request: Request, next: Next) -> Response {
    match admit(state, request).await {
        Ok(request) => {
            let mut response = next.run(request).await;
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
            response
        }
        Err(response) => *response,
    }
}

async fn admit(state: &Security, request: Request) -> Result<Request, Box<Response>> {
    if !allowed(request.headers(), &state.config) {
        return Err(Box::new(error(
            StatusCode::FORBIDDEN,
            "invalid_host_or_origin",
        )));
    }
    let Some(token) = bearer_token(request.headers()) else {
        return Err(Box::new(invalid_token(state)));
    };
    let verified = verify(state, token).await?;
    consume_quota(state, &verified).await?;
    let principal = apply_policy(state, &verified, token).await?;
    let mut request = if request.method() == Method::POST {
        screen_post_body(state, request, &principal).await?
    } else {
        request
    };
    request.extensions_mut().insert(principal);
    Ok(request)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let value = single_header(headers, header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    let (scheme, token) = value.split_once(' ')?;
    (scheme.eq_ignore_ascii_case("Bearer")
        && !token.is_empty()
        && token.len() <= MAX_BEARER_TOKEN_BYTES
        && !token.bytes().any(|byte| byte.is_ascii_whitespace()))
    .then_some(token)
}

fn invalid_token(state: &Security) -> Response {
    challenge(
        state,
        StatusCode::UNAUTHORIZED,
        "invalid_token",
        &state.services.scopes(),
    )
}

async fn verify(state: &Security, token: &str) -> Result<VerifiedPrincipal, Box<Response>> {
    match state.verifier.verify(token).await {
        Ok(principal) => Ok(principal),
        Err(cause) if cause.is_unavailable() => {
            tracing::error!(error = %cause, "identity verification infrastructure failed");
            let mut response = error(StatusCode::SERVICE_UNAVAILABLE, "identity_unavailable");
            response.headers_mut().insert(
                header::RETRY_AFTER,
                HeaderValue::from(IDENTITY_UNAVAILABLE_RETRY_AFTER_SECONDS),
            );
            Err(Box::new(response))
        }
        Err(cause) if cause.is_misconfigured() => {
            tracing::error!(error = %cause, "identity verification infrastructure failed");
            Err(Box::new(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "identity_misconfigured",
            )))
        }
        Err(_) => Err(Box::new(invalid_token(state))),
    }
}

async fn consume_quota(
    state: &Security,
    principal: &VerifiedPrincipal,
) -> Result<(), Box<Response>> {
    let key = format!(
        "mcp:{}:{}:{}",
        state.config.resource_url,
        principal.issuer().unwrap_or_default(),
        principal.subject()
    );
    let Ok(decision) = state.store.check_and_consume(&key, state.quota).await else {
        return Err(Box::new(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "rate_limit_unavailable",
        )));
    };
    metrics::counter!("http_rate_limit_decisions_total", "scope" => "mcp", "outcome" => if decision.allowed { "allowed" } else { "limited" }).increment(1);
    if decision.allowed {
        return Ok(());
    }
    let mut response = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
    response.headers_mut().insert(
        header::RETRY_AFTER,
        HeaderValue::from(decision.retry_after.as_secs().max(1)),
    );
    Err(Box::new(response))
}

async fn apply_policy(
    state: &Security,
    verified: &VerifiedPrincipal,
    token: &str,
) -> Result<Principal, Box<Response>> {
    let started = Instant::now();
    let result = state.policy.authenticate(verified, token).await;
    let outcome = match &result {
        Ok(_) => "allowed",
        Err(PolicyDenial::Inactive) => "inactive",
        Err(PolicyDenial::InsufficientScope(_)) => "insufficient_scope",
        Err(PolicyDenial::Unavailable) => "unavailable",
        Err(PolicyDenial::RateLimited(_)) => "limited",
    };
    metrics::counter!("mcp_authentication_policy_decisions_total", "outcome" => outcome)
        .increment(1);
    metrics::histogram!("mcp_authentication_policy_duration_seconds")
        .record(started.elapsed().as_secs_f64());
    result.map_err(|denial| Box::new(policy_denial(state, denial)))
}

/// Buffers a POST body to reject batches, answer cancellation notifications, and check tool scopes.
async fn screen_post_body(
    state: &Security,
    request: Request,
    principal: &Principal,
) -> Result<Request, Box<Response>> {
    let (parts, body) = request.into_parts();
    let Ok(bytes) = to_bytes(body, state.config.max_request_body_bytes).await else {
        return Err(Box::new(error(
            StatusCode::PAYLOAD_TOO_LARGE,
            "request_too_large",
        )));
    };
    if let Ok(message) = serde_json::from_slice::<Value>(&bytes) {
        if message.is_array() {
            return Err(Box::new(error(
                StatusCode::BAD_REQUEST,
                "batch_not_supported",
            )));
        }
        if is_cancel_notification(&message) {
            if let Some(id) = message
                .pointer("/params/requestId")
                .and_then(|id| serde_json::from_value(id.clone()).ok())
            {
                state.services.requests.cancel(principal, &id);
            }
            return Err(Box::new(
                (StatusCode::ACCEPTED, [(header::CACHE_CONTROL, "no-store")]).into_response(),
            ));
        }
        if let Some(scopes) = state.services.required_scopes(&message)
            && !permitted_scopes(principal, &scopes)
        {
            return Err(Box::new(challenge(
                state,
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                &scopes,
            )));
        }
    }
    Ok(Request::from_parts(parts, Body::from(bytes)))
}

fn is_cancel_notification(message: &Value) -> bool {
    message.get("jsonrpc").and_then(Value::as_str) == Some("2.0")
        && message.get("id").is_none()
        && message.get("method").and_then(Value::as_str) == Some("notifications/cancelled")
}

fn policy_denial(state: &Security, denial: PolicyDenial) -> Response {
    match denial {
        PolicyDenial::Inactive => challenge(
            state,
            StatusCode::UNAUTHORIZED,
            "invalid_token",
            &state.services.scopes(),
        ),
        PolicyDenial::InsufficientScope(scopes) => {
            if scopes.is_empty() || scopes.iter().any(|scope| !valid_scope(scope)) {
                return error(StatusCode::INTERNAL_SERVER_ERROR, "invalid_policy_scopes");
            }
            challenge(state, StatusCode::FORBIDDEN, "insufficient_scope", &scopes)
        }
        PolicyDenial::Unavailable => error(
            StatusCode::SERVICE_UNAVAILABLE,
            "authentication_policy_unavailable",
        ),
        PolicyDenial::RateLimited(retry_after) => {
            let mut response = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
            if let Ok(value) = HeaderValue::from_str(&retry_after.as_secs().max(1).to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
            response
        }
    }
}

fn challenge(state: &Security, status: StatusCode, code: &str, scopes: &[String]) -> Response {
    let value = format!(
        "Bearer resource_metadata=\"{}\", error=\"{code}\", scope=\"{}\"",
        state.metadata_url,
        scopes.join(" ")
    );
    let mut response = error(status, code);
    match HeaderValue::from_str(&value) {
        Ok(value) => {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
            response
        }
        Err(_) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "invalid_challenge_configuration",
        ),
    }
}

fn error(status: StatusCode, code: &str) -> Response {
    (
        status,
        [(header::CACHE_CONTROL, "no-store")],
        Json(json!({"error": code})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_and_origin_are_exact_and_duplicates_fail_closed() {
        let config = McpConfig {
            allowed_hosts: vec!["mcp.example:443".into()],
            allowed_origins: vec!["https://client.example".into()],
            ..Default::default()
        };
        let mut headers = HeaderMap::new();
        assert!(!allowed(&headers, &config));
        headers.insert(header::HOST, HeaderValue::from_static("MCP.EXAMPLE:443"));
        assert!(allowed(&headers, &config));
        for host in [
            "mcp.example",
            "evil.example:443",
            "mcp.example:443.evil",
            "user@mcp.example:443",
        ] {
            headers.insert(header::HOST, HeaderValue::from_str(host).expect("header"));
            assert!(!allowed(&headers, &config), "{host}");
        }
        headers.insert(header::HOST, HeaderValue::from_static("mcp.example:443"));
        headers.insert(
            header::ORIGIN,
            HeaderValue::from_static("https://client.example"),
        );
        assert!(allowed(&headers, &config));
        headers.append(
            header::ORIGIN,
            HeaderValue::from_static("https://client.example"),
        );
        assert!(!allowed(&headers, &config));
        headers.remove(header::ORIGIN);
        for origin in [
            "null",
            "https://client.example.evil",
            "http://client.example",
            "https://client.example/",
        ] {
            headers.insert(
                header::ORIGIN,
                HeaderValue::from_str(origin).expect("header"),
            );
            assert!(!allowed(&headers, &config), "{origin}");
        }
        headers.remove(header::ORIGIN);
        headers.append(header::HOST, HeaderValue::from_static("mcp.example:443"));
        assert!(!allowed(&headers, &config));
    }
}
