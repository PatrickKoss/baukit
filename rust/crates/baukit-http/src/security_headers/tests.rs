use std::time::Duration;

use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use serde_json::json;
use tower::ServiceExt as _;

use crate::{HttpOptions, finalize};

const ORIGIN: &str = "https://app.example.com";
const BASE_HEADERS: [(header::HeaderName, &str); 4] = [
    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
    (header::REFERRER_POLICY, "no-referrer"),
    (header::X_FRAME_OPTIONS, "DENY"),
    (
        header::CONTENT_SECURITY_POLICY,
        "default-src 'none'; frame-ancestors 'none'",
    ),
];

fn app(options: HttpOptions) -> Router {
    finalize(
        Router::new()
            .route(
                "/items",
                get(|| async { axum::Json(json!({ "ok": true })) }),
            )
            .route("/upload", post(|body: String| async move { body }))
            .route(
                "/limited",
                get(|| async { StatusCode::TOO_MANY_REQUESTS.into_response() }),
            )
            .route(
                "/cached",
                get(|| async { ([(header::CACHE_CONTROL, "public, max-age=300")], "cached") }),
            )
            .route(
                "/auth/session/refresh",
                post(|| async { ([(header::CACHE_CONTROL, "public, max-age=60")], "ok") }),
            )
            .route(
                "/docs",
                get(|| async {
                    (
                        [(header::CONTENT_SECURITY_POLICY, "default-src 'self'")],
                        Html("<p>docs</p>"),
                    )
                }),
            ),
        options,
    )
}

fn origin_options() -> HttpOptions {
    HttpOptions::default()
        .with_allowed_origins([ORIGIN])
        .expect("valid origin")
}

async fn send(router: Router, request: Request<Body>) -> Response {
    router.oneshot(request).await.expect("response")
}

fn get_request(uri: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

fn assert_base_headers(response: &Response) {
    for (name, value) in BASE_HEADERS {
        assert_eq!(response.headers()[&name], value, "{name}");
    }
}

#[tokio::test]
async fn json_route_has_every_base_header() {
    let response = send(app(HttpOptions::default()), get_request("/items")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_base_headers(&response);
    assert!(
        !response
            .headers()
            .contains_key(header::STRICT_TRANSPORT_SECURITY)
    );
}

#[tokio::test]
async fn error_responses_have_every_base_header() {
    let not_found = send(app(HttpOptions::default()), get_request("/missing")).await;
    assert_eq!(not_found.status(), StatusCode::NOT_FOUND);
    assert_base_headers(&not_found);

    let limited = send(app(HttpOptions::default()), get_request("/limited")).await;
    assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_base_headers(&limited);

    let options = HttpOptions {
        body_size_limit: 4,
        ..HttpOptions::default()
    };
    let too_large = send(
        app(options),
        Request::builder()
            .method(Method::POST)
            .uri("/upload")
            .header(header::CONTENT_LENGTH, "16")
            .body(Body::from("0123456789abcdef"))
            .expect("request"),
    )
    .await;
    assert_eq!(too_large.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_base_headers(&too_large);
}

fn preflight(request_headers: &str) -> Request<Body> {
    Request::builder()
        .method(Method::OPTIONS)
        .uri("/auth/session/refresh")
        .header(header::ORIGIN, ORIGIN)
        .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .header(header::ACCESS_CONTROL_REQUEST_HEADERS, request_headers)
        .body(Body::empty())
        .expect("request")
}

#[tokio::test]
async fn preflight_accepts_the_session_header_and_has_base_headers() {
    let response = send(app(origin_options()), preflight("x-baukit-session")).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        ORIGIN
    );
    let allowed = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .expect("ascii");
    assert!(
        allowed
            .split(',')
            .any(|name| name.trim().eq_ignore_ascii_case("x-baukit-session")),
        "{allowed}"
    );
    assert_base_headers(&response);
}

#[tokio::test]
async fn authorized_requests_get_no_store_over_a_handler_cache_control() {
    let response = send(
        app(HttpOptions::default()),
        Request::builder()
            .uri("/cached")
            .header(header::AUTHORIZATION, "Bearer token")
            .body(Body::empty())
            .expect("request"),
    )
    .await;

    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn anonymous_requests_keep_the_handler_cache_control() {
    let response = send(app(HttpOptions::default()), get_request("/cached")).await;

    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "public, max-age=300"
    );
}

#[tokio::test]
async fn session_routes_get_no_store() {
    let response = send(
        app(HttpOptions::default()),
        Request::builder()
            .method(Method::POST)
            .uri("/auth/session/refresh")
            .body(Body::empty())
            .expect("request"),
    )
    .await;

    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn existing_no_store_is_kept() {
    let response = send(
        app(HttpOptions::default()),
        Request::builder()
            .uri("/items")
            .header(header::AUTHORIZATION, "Bearer token")
            .body(Body::empty())
            .expect("request"),
    )
    .await;

    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
}

#[tokio::test]
async fn handler_csp_is_preserved_for_html() {
    let response = send(app(HttpOptions::default()), get_request("/docs")).await;

    assert_eq!(
        response.headers()[header::CONTENT_SECURITY_POLICY],
        "default-src 'self'"
    );
    assert_eq!(response.headers()[header::X_FRAME_OPTIONS], "DENY");
}

fn host_request(host: &str) -> Request<Body> {
    Request::builder()
        .uri("/items")
        .header(header::HOST, host)
        .body(Body::empty())
        .expect("request")
}

#[tokio::test]
async fn hsts_is_sent_only_when_enabled_and_not_for_loopback_hosts() {
    let config = baukit_config::HttpConfig {
        hsts_max_age: Some(baukit_config::DEFAULT_HSTS_MAX_AGE),
        ..baukit_config::HttpConfig::default()
    };
    let options = HttpOptions::from_config(&config).expect("valid options");
    assert_eq!(
        options.hsts_max_age(),
        Some(Duration::from_secs(31_536_000))
    );

    let public = send(app(options.clone()), host_request("api.example.com")).await;
    assert_eq!(
        public.headers()[header::STRICT_TRANSPORT_SECURITY],
        "max-age=31536000"
    );

    for host in ["localhost:8080", "127.0.0.1", "[::1]:3000", "api.localhost"] {
        let local = send(app(options.clone()), host_request(host)).await;
        assert!(
            !local
                .headers()
                .contains_key(header::STRICT_TRANSPORT_SECURITY),
            "{host}"
        );
    }

    let disabled = send(app(HttpOptions::default()), host_request("api.example.com")).await;
    assert!(
        !disabled
            .headers()
            .contains_key(header::STRICT_TRANSPORT_SECURITY)
    );
    assert_eq!(HttpOptions::default().hsts_max_age(), None);
}
