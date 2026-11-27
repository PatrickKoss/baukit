use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Mutex, time::Duration};

use axum::{
    body::{Body, to_bytes},
    http::{
        Method, Request, StatusCode,
        header::{self, RETRY_AFTER},
    },
    middleware,
    routing::{any, get},
};
use baukit_auth::{AuthState, IdentityVerifier, Principal, VerificationError, establish_principal};
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
    ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>> {
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
fn xff_resolution_reads_only_the_entry_at_len_minus_hops() {
    let mut headers = HeaderMap::new();
    headers.insert(
        X_FORWARDED_FOR,
        HeaderValue::from_static("198.51.100.7, 10.0.0.4"),
    );
    let peer = Some("10.0.0.5:443".parse().expect("peer"));
    let ip = |text: &str| Some(text.parse::<IpAddr>().expect("IP"));
    assert_eq!(resolve_client_ip(&headers, peer, 1), ip("10.0.0.4"));
    assert_eq!(resolve_client_ip(&headers, peer, 2), ip("198.51.100.7"));
    assert_eq!(resolve_client_ip(&headers, peer, 0), ip("10.0.0.5"));
    assert_eq!(resolve_client_ip(&headers, peer, 3), ip("10.0.0.5"));
    headers.insert(X_FORWARDED_FOR, HeaderValue::from_static("not-an-ip"));
    assert_eq!(resolve_client_ip(&headers, peer, 1), ip("10.0.0.5"));
}

#[test]
fn client_written_xff_garbage_does_not_move_the_bucket_to_the_proxy() {
    let peer = Some("10.0.0.5:443".parse().expect("peer"));
    let mut headers = HeaderMap::new();
    headers.append(X_FORWARDED_FOR, HeaderValue::from_static("x, ::not-ip"));
    headers.append(X_FORWARDED_FOR, HeaderValue::from_static("203.0.113.9"));
    assert_eq!(
        resolve_client_ip(&headers, peer, 1),
        Some("203.0.113.9".parse().expect("IP"))
    );
    let mut invalid_utf8 = HeaderMap::new();
    invalid_utf8.append(
        X_FORWARDED_FOR,
        HeaderValue::from_bytes(b"\xfe\xff, 203.0.113.9").expect("header"),
    );
    assert_eq!(
        resolve_client_ip(&invalid_utf8, peer, 1),
        Some("203.0.113.9".parse().expect("IP"))
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
async fn missing_peer_fails_closed_instead_of_sharing_an_unknown_bucket() {
    let store = RecordingStore::default();
    let app = layers(
        Router::new().route("/", get(|| async { "unreachable" })),
        store.clone(),
        RateLimitOptions::default(),
    );
    let request = Request::builder()
        .uri("/")
        .body(Body::empty())
        .expect("request");

    let response = app.oneshot(request).await.expect("response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert!(store.counts().is_empty());
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
async fn identities_and_named_groups_use_independent_counters() {
    let store = RecordingStore::default();
    let rate_limit = RateLimitOptions::default();
    let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
    let first = authenticated_route_group(
        Router::new().route("/first", any(|| async { "first" })),
        store.clone(),
        AuthenticatedRouteGroupOptions::new("first", quota, &rate_limit).expect("group"),
        |principal: &Principal| format!("account:{}", principal.subject()),
        |_| true,
    );
    let second = authenticated_route_group(
        Router::new().route("/second", any(|| async { "second" })),
        store.clone(),
        AuthenticatedRouteGroupOptions::new("second", quota, &rate_limit).expect("group"),
        |principal: &Principal| format!("account:{}", principal.subject()),
        |_| true,
    );
    let app = first.merge(second);

    for (path, subject, expected) in [
        ("/first", "alice", StatusCode::OK),
        ("/first", "alice", StatusCode::TOO_MANY_REQUESTS),
        ("/second", "alice", StatusCode::OK),
        ("/first", "bob", StatusCode::OK),
        ("/second", "bob", StatusCode::OK),
    ] {
        let response = app
            .clone()
            .oneshot(request_with_principal(Method::GET, path, subject))
            .await
            .expect("response");
        assert_eq!(response.status(), expected, "{subject} {path}");
    }

    assert_eq!(
        store.counts(),
        BTreeMap::from([
            ("rl:group:first:account:alice".to_owned(), 2),
            ("rl:group:first:account:bob".to_owned(), 1),
            ("rl:group:second:account:alice".to_owned(), 1),
            ("rl:group:second:account:bob".to_owned(), 1),
        ])
    );
}

#[tokio::test]
async fn route_group_predicate_bypasses_requests_that_do_not_count() {
    let store = RecordingStore::default();
    let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
    let app = authenticated_route_group(
        Router::new().route("/items", any(|| async { "ok" })),
        store.clone(),
        AuthenticatedRouteGroupOptions::new("item_writes", quota, &RateLimitOptions::default())
            .expect("group"),
        |principal: &Principal| principal.subject().to_owned(),
        |request| request.method() != Method::GET,
    );

    for method in [Method::GET, Method::GET, Method::POST] {
        let response = app
            .clone()
            .oneshot(request_with_principal(method, "/items", "alice"))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }
    let response = app
        .oneshot(request_with_principal(Method::POST, "/items", "alice"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        store.counts(),
        BTreeMap::from([("rl:group:item_writes:alice".to_owned(), 2)])
    );
}

#[tokio::test]
async fn route_group_headers_and_body_use_the_same_retry_delay() {
    let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
    let app = authenticated_route_group(
        Router::new().route("/", get(|| async { "ok" })),
        InMemoryRateLimitStore::default(),
        AuthenticatedRouteGroupOptions::new("reads", quota, &RateLimitOptions::default())
            .expect("group"),
        |principal: &Principal| principal.subject().to_owned(),
        |_| true,
    );
    assert_eq!(
        app.clone()
            .oneshot(request_with_principal(Method::GET, "/", "private-subject"))
            .await
            .expect("response")
            .status(),
        StatusCode::OK
    );

    let response = app
        .oneshot(request_with_principal(Method::GET, "/", "private-subject"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    let retry_after = response.headers()[RETRY_AFTER]
        .to_str()
        .expect("retry header")
        .parse::<u64>()
        .expect("retry seconds");
    assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "1");
    assert_eq!(response.headers()[RATE_LIMIT_REMAINING], "0");
    assert_eq!(
        response.headers()[RATE_LIMIT_RESET],
        retry_after.to_string()
    );
    let json: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
    )
    .expect("JSON");
    assert_eq!(json["error"]["code"], "rate_limited");
    assert_eq!(json["error"]["details"]["retryAfter"], retry_after);
}

#[tokio::test]
async fn route_group_store_errors_follow_the_configured_fail_mode() {
    for (fail_mode, expected) in [
        (RateLimitFailMode::Open, StatusCode::OK),
        (RateLimitFailMode::Closed, StatusCode::TOO_MANY_REQUESTS),
    ] {
        let mut rate_limit = RateLimitOptions::default();
        rate_limit.fail_mode = fail_mode;
        let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
        let app = authenticated_route_group(
            Router::new().route("/", get(|| async { "ok" })),
            FailingStore,
            AuthenticatedRouteGroupOptions::new("reads", quota, &rate_limit).expect("group"),
            |principal: &Principal| principal.subject().to_owned(),
            |_| true,
        );
        let response = app
            .oneshot(request_with_principal(Method::GET, "/", "alice"))
            .await
            .expect("response");
        assert_eq!(response.status(), expected);
        if fail_mode == RateLimitFailMode::Closed {
            assert_eq!(response.headers()[RETRY_AFTER], "60");
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn identity_values_do_not_become_metric_labels() {
    use metrics_util::debugging::DebuggingRecorder;

    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _guard = metrics::set_default_local_recorder(&recorder);
    let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
    let app = authenticated_route_group(
        Router::new().route("/", get(|| async { "ok" })),
        InMemoryRateLimitStore::default(),
        AuthenticatedRouteGroupOptions::new("private_reads", quota, &RateLimitOptions::default())
            .expect("group"),
        |principal: &Principal| principal.subject().to_owned(),
        |_| true,
    );
    let response = app
        .oneshot(request_with_principal(Method::GET, "/", "private-user-42"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);

    let metrics = snapshotter.snapshot().into_vec();
    let metric = metrics
        .iter()
        .find(|(key, _, _, _)| key.key().name() == HTTP_RATE_LIMIT_DECISIONS_TOTAL)
        .expect("rate-limit decision metric");
    let labels = metric
        .0
        .key()
        .labels()
        .map(|label| (label.key(), label.value()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(labels.get("scope"), Some(&"private_reads"));
    assert_eq!(labels.get("outcome"), Some(&"allowed"));
    assert_eq!(labels.len(), 2);
    assert!(!format!("{:?}", metric.0).contains("private-user-42"));
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
    assert_eq!(json["error"]["details"]["retryAfter"], 60);
}

fn nested_identity_inside_ip(store: InMemoryRateLimitStore) -> Router {
    let mut identity = RateLimitOptions::default();
    identity.ip.enabled = false;
    identity.identity.quota = Quota::new(2, Duration::from_secs(30), 0).expect("quota");
    let mut ip = RateLimitOptions::default();
    ip.identity.enabled = false;
    ip.ip.quota = Quota::new(10, Duration::from_secs(60), 0).expect("quota");
    let auth = AuthState::new(SubjectVerifier);
    let inner = layers(
        Router::new().route("/", get(|_principal: Principal| async { "ok" })),
        store.clone(),
        identity,
    )
    .with_state(auth.clone())
    .layer(middleware::from_fn_with_state(auth, establish_principal));
    layers(inner, store, ip)
}

#[tokio::test]
async fn nested_ip_layer_keeps_inner_identity_headers() {
    let app = nested_identity_inside_ip(InMemoryRateLimitStore::default());

    let response = app
        .oneshot(authenticated_request("alice"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "2");
    assert_eq!(response.headers()[RATE_LIMIT_REMAINING], "1");
}

#[tokio::test]
async fn nested_identity_rejection_keeps_identity_headers() {
    let app = nested_identity_inside_ip(InMemoryRateLimitStore::default());
    for _ in 0..2 {
        app.clone()
            .oneshot(authenticated_request("alice"))
            .await
            .expect("response");
    }

    let response = app
        .oneshot(authenticated_request("alice"))
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "2");
    assert_eq!(response.headers()[RATE_LIMIT_REMAINING], "0");
    assert_eq!(response.headers()[RETRY_AFTER], "15");
}

#[tokio::test]
async fn ip_layer_sets_headers_only_when_none_exist() {
    let mut options = RateLimitOptions::default();
    options.identity.enabled = false;
    options.ip.quota = Quota::new(10, Duration::from_secs(60), 0).expect("quota");
    let bare = layers(
        Router::new().route("/", get(|| async { "ok" })),
        InMemoryRateLimitStore::default(),
        options.clone(),
    );
    let preset = layers(
        Router::new().route("/", get(|| async { ([(RATE_LIMIT_LIMIT, "7")], "ok") })),
        InMemoryRateLimitStore::default(),
        options,
    );

    let response = bare.oneshot(request_with_peer()).await.expect("response");
    assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "10");
    assert_eq!(response.headers()[RATE_LIMIT_REMAINING], "9");

    let response = preset.oneshot(request_with_peer()).await.expect("response");
    assert_eq!(response.headers()[RATE_LIMIT_LIMIT], "7");
    assert!(!response.headers().contains_key(RATE_LIMIT_REMAINING));
}

const APP_ORIGIN: &str = "https://app.example.com";

fn cross_origin_request(method: Method) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri("/")
        .header(header::ORIGIN, APP_ORIGIN)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
        .body(Body::empty())
        .expect("request");
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:1234".parse::<SocketAddr>().expect("peer"),
    ));
    request
}

fn exposed_headers(response: &Response) -> Vec<String> {
    response
        .headers()
        .get_all(header::ACCESS_CONTROL_EXPOSE_HEADERS)
        .iter()
        .flat_map(|value| value.to_str().expect("exposed headers").split(','))
        .map(|name| name.trim().to_ascii_lowercase())
        .collect()
}

#[tokio::test]
async fn browsers_can_read_rate_limit_headers_through_the_http_layers() {
    let mut options = RateLimitOptions::default();
    options.identity.enabled = false;
    options.ip.quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
    let limited = layers(
        Router::new().route("/", get(|| async { "ok" })),
        InMemoryRateLimitStore::default(),
        options,
    );
    let http_options = baukit_http::HttpOptions::default()
        .with_allowed_origins([APP_ORIGIN])
        .expect("valid origin");
    let app = baukit_http::finalize(limited, http_options);

    let preflight = app
        .clone()
        .oneshot(cross_origin_request(Method::OPTIONS))
        .await
        .expect("preflight");
    assert_eq!(preflight.status(), StatusCode::OK);
    assert_eq!(
        preflight.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        APP_ORIGIN
    );
    assert!(!preflight.headers().contains_key(RATE_LIMIT_LIMIT));

    let first = app
        .clone()
        .oneshot(cross_origin_request(Method::GET))
        .await
        .expect("response");
    assert_eq!(first.status(), StatusCode::OK);

    let rejected = app
        .oneshot(cross_origin_request(Method::GET))
        .await
        .expect("response");
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        rejected.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        APP_ORIGIN
    );
    assert_eq!(
        rejected.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    let exposed = exposed_headers(&rejected);
    for emitted in [
        RETRY_AFTER,
        RATE_LIMIT_LIMIT,
        RATE_LIMIT_REMAINING,
        RATE_LIMIT_RESET,
    ] {
        assert!(
            rejected.headers().contains_key(&emitted),
            "missing emitted {emitted}"
        );
        assert!(
            exposed.iter().any(|name| name == emitted.as_str()),
            "{emitted} is not exposed in {exposed:?}"
        );
    }
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

fn request_with_principal(method: Method, path: &str, subject: &str) -> Request<Body> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .expect("request");
    request.extensions_mut().insert(Principal::new(subject));
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
