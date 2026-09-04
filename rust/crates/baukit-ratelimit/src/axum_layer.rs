use std::{net::IpAddr, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue, header::RETRY_AFTER},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use baukit_auth::Principal;

use crate::{
    Quota, RateLimitDecision, RateLimitFailMode, RateLimitOptions, RateLimitScopeOptions,
    RateLimitStore,
};

/// Counter for rate-limit decisions, labeled exactly `scope` and `outcome`.
pub const HTTP_RATE_LIMIT_DECISIONS_TOTAL: &str = "http_rate_limit_decisions_total";
/// IETF rate-limit policy limit header.
pub const RATE_LIMIT_LIMIT: HeaderName = HeaderName::from_static("ratelimit-limit");
/// IETF rate-limit remaining-quota header.
pub const RATE_LIMIT_REMAINING: HeaderName = HeaderName::from_static("ratelimit-remaining");
/// IETF rate-limit reset-delay header.
pub const RATE_LIMIT_RESET: HeaderName = HeaderName::from_static("ratelimit-reset");

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

#[derive(Clone)]
struct LayerState {
    store: Arc<dyn RateLimitStore>,
    options: RateLimitOptions,
}

/// Applies identity-first and client-IP rate limiting to an Axum router.
///
/// Compose this function independently with [`baukit_http::layers`]. A verified
/// [`Principal`] already present in request extensions activates the identity
/// scope; the IP scope is evaluated for every request when enabled.
pub fn layers<S, Store>(router: Router<S>, store: Store, options: RateLimitOptions) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Store: RateLimitStore + 'static,
{
    router.layer(middleware::from_fn_with_state(
        LayerState {
            store: Arc::new(store),
            options,
        },
        rate_limit_request,
    ))
}

async fn rate_limit_request(
    State(state): State<LayerState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response_policy = None;

    if state.options.identity.enabled
        && let Some(principal) = request.extensions().get::<Principal>()
    {
        let key = state.options.identity_key(principal.subject());
        match evaluate(&state, "identity", &key, state.options.identity).await {
            Evaluation::Allowed(decision) => {
                response_policy = Some((state.options.identity, decision))
            }
            Evaluation::PassAfterError => {}
            Evaluation::Rejected(response) => return response,
        }
    }

    if state.options.ip.enabled {
        let peer = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|connect_info| connect_info.0);
        let address = resolve_client_ip(request.headers(), peer, state.options.trusted_proxy_hops);
        let key = state.options.ip_key(address);
        match evaluate(&state, "ip", &key, state.options.ip).await {
            Evaluation::Allowed(decision) => {
                if response_policy.is_none() {
                    response_policy = Some((state.options.ip, decision));
                }
            }
            Evaluation::PassAfterError => {}
            Evaluation::Rejected(response) => return response,
        }
    }

    let mut response = next.run(request).await;
    if let Some((scope, decision)) = response_policy {
        insert_rate_limit_headers(
            response.headers_mut(),
            scope.quota,
            decision,
            scope.quota.period(),
        );
    }
    response
}

enum Evaluation {
    Allowed(RateLimitDecision),
    PassAfterError,
    Rejected(Response),
}

async fn evaluate(
    state: &LayerState,
    scope: &'static str,
    key: &str,
    options: RateLimitScopeOptions,
) -> Evaluation {
    match state.store.check_and_consume(key, options.quota).await {
        Ok(decision) if decision.allowed => {
            record(scope, "allowed");
            Evaluation::Allowed(decision)
        }
        Ok(decision) => {
            record(scope, "limited");
            Evaluation::Rejected(limited_response(options.quota, decision))
        }
        Err(error) => {
            record(scope, "error");
            tracing::warn!(scope, error = %error, "rate-limit store decision failed");
            match state.options.fail_mode {
                RateLimitFailMode::Open => Evaluation::PassAfterError,
                RateLimitFailMode::Closed => Evaluation::Rejected(limited_response(
                    options.quota,
                    RateLimitDecision {
                        allowed: false,
                        remaining: 0,
                        retry_after: options.quota.period(),
                    },
                )),
            }
        }
    }
}

fn limited_response(quota: Quota, decision: RateLimitDecision) -> Response {
    let mut response = baukit_http::ApiError::rate_limited().into_response();
    insert_rate_limit_headers(
        response.headers_mut(),
        quota,
        decision,
        decision.retry_after,
    );
    response.headers_mut().insert(
        RETRY_AFTER,
        number_header(duration_seconds_ceil(decision.retry_after)),
    );
    response
}

fn insert_rate_limit_headers(
    headers: &mut HeaderMap,
    quota: Quota,
    decision: RateLimitDecision,
    reset: Duration,
) {
    headers.insert(RATE_LIMIT_LIMIT, number_header(quota.capacity()));
    headers.insert(RATE_LIMIT_REMAINING, number_header(decision.remaining));
    headers.insert(
        RATE_LIMIT_RESET,
        number_header(duration_seconds_ceil(reset)),
    );
}

fn number_header(value: u64) -> HeaderValue {
    HeaderValue::from_str(&value.to_string()).expect("unsigned integers are valid header values")
}

fn duration_seconds_ceil(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}

fn record(scope: &'static str, outcome: &'static str) {
    metrics::counter!(
        HTTP_RATE_LIMIT_DECISIONS_TOTAL,
        "scope" => scope,
        "outcome" => outcome
    )
    .increment(1);
}

/// Resolves a client IP without trusting more of `X-Forwarded-For` than configured.
///
/// `trusted_proxy_hops` counts the socket peer. With the default of one, the
/// rightmost XFF address is selected. Zero ignores XFF. A missing socket peer,
/// malformed chain, or chain shorter than the configured trust depth falls
/// back to the socket peer when available.
#[must_use]
pub fn resolve_client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxy_hops: usize,
) -> Option<IpAddr> {
    let peer_ip = peer.map(|address| address.ip());
    if trusted_proxy_hops == 0 || peer.is_none() {
        return peer_ip;
    }
    let mut addresses = Vec::new();
    for value in headers.get_all(&X_FORWARDED_FOR) {
        let Ok(value) = value.to_str() else {
            return peer_ip;
        };
        for address in value.split(',') {
            let Ok(address) = address.trim().parse() else {
                return peer_ip;
            };
            addresses.push(address);
        }
    }
    addresses
        .len()
        .checked_sub(trusted_proxy_hops)
        .and_then(|index| addresses.get(index).copied())
        .or(peer_ip)
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Mutex, time::Duration};

    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
        middleware,
        routing::get,
    };
    use baukit_auth::{
        AuthState, IdentityVerifier, Principal, VerificationError, establish_principal,
    };
    use serde_json::Value;
    use tower::ServiceExt as _;

    use super::*;
    use crate::{InMemoryRateLimitStore, RateLimitStoreError};

    #[derive(Clone, Default)]
    struct RecordingStore {
        counts: Arc<Mutex<BTreeMap<String, u64>>>,
    }

    impl RecordingStore {
        fn counts(&self) -> BTreeMap<String, u64> {
            self.counts.lock().expect("counts lock").clone()
        }
    }

    impl RateLimitStore for RecordingStore {
        fn check_and_consume<'a>(
            &'a self,
            key: &'a str,
            quota: Quota,
        ) -> Pin<Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>>
        {
            let consumed = {
                let mut counts = self.counts.lock().expect("counts lock");
                let count = counts.entry(key.to_owned()).or_default();
                *count += 1;
                *count
            };
            Box::pin(async move {
                if consumed <= quota.capacity() {
                    Ok(RateLimitDecision::allowed(quota.capacity() - consumed))
                } else {
                    Ok(RateLimitDecision::limited(0, quota.period()))
                }
            })
        }
    }

    #[derive(Clone, Copy)]
    struct SubjectVerifier;

    impl IdentityVerifier for SubjectVerifier {
        fn verify<'a>(
            &'a self,
            token: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>>
        {
            Box::pin(async move {
                match token {
                    "alice" | "bob" => Ok(Principal::new(token)),
                    "expired" => Err(VerificationError::Expired),
                    _ => Err(VerificationError::InvalidSignature),
                }
            })
        }
    }

    #[test]
    fn xff_resolution_honors_trusted_hop_count() {
        let mut headers = HeaderMap::new();
        headers.insert(
            X_FORWARDED_FOR,
            HeaderValue::from_static("198.51.100.7, 10.0.0.4"),
        );
        let peer = Some("10.0.0.5:443".parse().expect("peer"));
        assert_eq!(
            resolve_client_ip(&headers, peer, 1),
            Some("10.0.0.4".parse().expect("IP"))
        );
        assert_eq!(
            resolve_client_ip(&headers, peer, 2),
            Some("198.51.100.7".parse().expect("IP"))
        );
        assert_eq!(
            resolve_client_ip(&headers, peer, 0),
            Some("10.0.0.5".parse().expect("IP"))
        );
        headers.insert(X_FORWARDED_FOR, HeaderValue::from_static("not-an-ip"));
        assert_eq!(
            resolve_client_ip(&headers, peer, 1),
            Some("10.0.0.5".parse().expect("IP"))
        );
    }

    #[tokio::test]
    async fn middleware_principals_use_separate_identity_buckets() {
        let store = RecordingStore::default();
        let mut options = RateLimitOptions::default();
        options.identity.quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
        options.ip.enabled = false;
        let auth = AuthState::new(SubjectVerifier);
        let app = layers(
            Router::new().route("/", get(|_principal: Principal| async { "ok" })),
            store.clone(),
            options,
        )
        .with_state(auth.clone())
        .layer(middleware::from_fn_with_state(auth, establish_principal));

        assert_eq!(
            app.clone()
                .oneshot(authenticated_request("alice"))
                .await
                .expect("response")
                .status(),
            StatusCode::OK
        );
        assert_eq!(
            app.clone()
                .oneshot(authenticated_request("alice"))
                .await
                .expect("response")
                .status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        assert_eq!(
            app.oneshot(authenticated_request("bob"))
                .await
                .expect("response")
                .status(),
            StatusCode::OK
        );
        assert_eq!(store.counts()["rl:id:alice"], 2);
        assert_eq!(store.counts()["rl:id:bob"], 1);
    }

    #[tokio::test]
    async fn anonymous_requests_consume_only_the_ip_bucket() {
        let store = RecordingStore::default();
        let app = layers(
            Router::new().route("/", get(|| async { "anonymous" })),
            store.clone(),
            RateLimitOptions::default(),
        )
        .layer(middleware::from_fn_with_state(
            AuthState::new(SubjectVerifier),
            establish_principal,
        ));

        let response = app.oneshot(request_with_peer()).await.expect("response");

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            store.counts(),
            BTreeMap::from([("rl:ip:192.0.2.10".to_owned(), 1)])
        );
    }

    #[tokio::test]
    async fn rejected_credentials_do_not_consume_anonymous_buckets() {
        for token in ["invalid", "expired"] {
            let store = RecordingStore::default();
            let app = layers(
                Router::new().route("/", get(|| async { "unreachable" })),
                store.clone(),
                RateLimitOptions::default(),
            )
            .layer(middleware::from_fn_with_state(
                AuthState::new(SubjectVerifier),
                establish_principal,
            ));

            let response = app
                .oneshot(authenticated_request(token))
                .await
                .expect("response");

            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert!(store.counts().is_empty());
        }
    }

    #[tokio::test]
    async fn limited_response_uses_standard_envelope_and_headers() {
        let mut options = RateLimitOptions::default();
        options.identity.enabled = false;
        options.ip.quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
        let app = layers(
            Router::new().route("/", get(|| async { "ok" })),
            InMemoryRateLimitStore::default(),
            options,
        );
        let first = app
            .clone()
            .oneshot(request_with_peer())
            .await
            .expect("response");
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[RATE_LIMIT_LIMIT], "1");
        assert_eq!(first.headers()[RATE_LIMIT_REMAINING], "0");

        let response = app.oneshot(request_with_peer()).await.expect("response");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()[RETRY_AFTER], "60");
        assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "1");
        assert_eq!(response.headers()[RATE_LIMIT_REMAINING], "0");
        assert_eq!(response.headers()[RATE_LIMIT_RESET], "60");
        let json: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("body"),
        )
        .expect("JSON");
        assert_eq!(json["error"]["code"], "rate_limited");
    }

    #[tokio::test]
    async fn store_errors_fail_open_or_closed() {
        let mut open = RateLimitOptions::default();
        open.identity.enabled = false;
        let app = layers(
            Router::new().route("/", get(|| async { "ok" })),
            FailingStore,
            open,
        );
        assert_eq!(
            app.oneshot(request_with_peer())
                .await
                .expect("response")
                .status(),
            StatusCode::OK
        );

        let mut closed = RateLimitOptions::default();
        closed.identity.enabled = false;
        closed.fail_mode = RateLimitFailMode::Closed;
        let app = layers(
            Router::new().route("/", get(|| async { "ok" })),
            FailingStore,
            closed,
        );
        let response = app.oneshot(request_with_peer()).await.expect("response");
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key(RETRY_AFTER));
        assert!(response.headers().contains_key(RATE_LIMIT_LIMIT));
    }

    fn request_with_peer() -> Request<Body> {
        let mut request = Request::builder()
            .uri("/")
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(
            "192.0.2.10:1234".parse::<SocketAddr>().expect("peer"),
        ));
        request
    }

    fn authenticated_request(token: &str) -> Request<Body> {
        let mut request = Request::builder()
            .uri("/")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .expect("request");
        request.extensions_mut().insert(ConnectInfo(
            "192.0.2.10:1234".parse::<SocketAddr>().expect("peer"),
        ));
        request
    }

    #[derive(Clone, Copy)]
    struct FailingStore;

    impl RateLimitStore for FailingStore {
        fn check_and_consume<'a>(
            &'a self,
            _key: &'a str,
            _quota: Quota,
        ) -> Pin<Box<dyn Future<Output = Result<RateLimitDecision, RateLimitStoreError>> + Send + 'a>>
        {
            Box::pin(async { Err(RateLimitStoreError::unavailable("fixture unavailable")) })
        }
    }
}
