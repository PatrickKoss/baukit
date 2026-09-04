use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, LazyLock, Mutex, Once},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::DefaultBodyLimit,
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::IntoResponse,
    routing::{any, get, post},
};
use metrics_util::{
    CompositeKey,
    debugging::{DebugValue, DebuggingRecorder},
};
use opentelemetry::{global, trace::TracerProvider as _};
use opentelemetry_sdk::{propagation::TraceContextPropagator, trace::SdkTracerProvider};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use tracing::{Subscriber, field::Visit};
use tracing_subscriber::{Layer, Registry, layer::SubscriberExt as _};
use utoipa::OpenApi as _;

use super::*;

#[allow(dead_code)]
#[derive(serde::Deserialize, utoipa::IntoParams)]
struct InferredListQuery {
    tag: Vec<String>,
}

#[utoipa::path(get, path = "/inferred-items", params(InferredListQuery))]
#[allow(dead_code)]
async fn inferred_list_query(Query(query): Query<InferredListQuery>) {
    let _ = query.tag;
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(inferred_list_query))]
struct InferredQueryApi;

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("response body");
    serde_json::from_slice(&bytes).expect("JSON response")
}

#[derive(serde::Deserialize)]
struct JsonInput {
    count: usize,
}

async fn json_input_handler(ApiJson(input): ApiJson<JsonInput>) -> String {
    input.count.to_string()
}

fn classified_json_options() -> HttpOptions {
    let codes = JsonRejectionCodes::new(
        "payload_too_large",
        "unsupported_media_type",
        "invalid_json",
        "validation_failed",
    )
    .expect("valid JSON rejection codes");
    HttpOptions::default().with_json_rejection_codes(codes)
}

async fn json_request(
    options: HttpOptions,
    content_type: Option<&str>,
    body: impl Into<Body>,
) -> axum::response::Response {
    let app = finalize(
        Router::new().route("/json", post(json_input_handler)),
        options,
    );
    let mut request = Request::builder().method(Method::POST).uri("/json");
    if let Some(content_type) = content_type {
        request = request.header(header::CONTENT_TYPE, content_type);
    }
    app.oneshot(request.body(body.into()).expect("request"))
        .await
        .expect("response")
}

#[tokio::test]
async fn request_id_is_passed_through_and_available_to_handlers() {
    async fn handler(request_id: RequestId) -> String {
        request_id.to_string()
    }

    let response = layers(
        Router::new().route("/request-id", get(handler)),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .uri("/request-id")
            .header(X_REQUEST_ID, "caller-id-123")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");

    assert_eq!(response.headers()[X_REQUEST_ID], "caller-id-123");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], b"caller-id-123");
}

#[tokio::test]
async fn request_id_is_generated_and_echoed() {
    async fn handler(request_id: RequestId) -> String {
        request_id.to_string()
    }

    let response = layers(
        Router::new().route("/request-id", get(handler)),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .uri("/request-id")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");

    let header_id = response.headers()[X_REQUEST_ID]
        .to_str()
        .expect("request ID header")
        .to_owned();
    assert!(uuid::Uuid::parse_str(&header_id).is_ok());
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    assert_eq!(&body[..], header_id.as_bytes());
}

#[tokio::test]
async fn error_envelope_has_the_exact_shared_shape() {
    async fn handler() -> Result<(), ApiError> {
        let details = BTreeMap::from([("email".to_owned(), json!(["is invalid"]))]);
        Err(ApiError::validation(details))
    }

    let response = layers(
        Router::new().route("/validate", post(handler)),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .method(Method::POST)
            .uri("/validate")
            .header(X_REQUEST_ID, "validation-request")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(response).await,
        json!({
            "error": {
                "code": "validation_failed",
                "message": "The request is invalid",
                "request_id": "validation-request",
                "details": {"email": ["is invalid"]}
            }
        })
    );
}

#[test]
fn api_error_stays_below_the_clippy_large_error_threshold() {
    const CLIPPY_LARGE_ERROR_THRESHOLD: usize = 128;
    assert!(std::mem::size_of::<ApiError>() <= CLIPPY_LARGE_ERROR_THRESHOLD);
}

#[tokio::test]
async fn api_error_adds_headers_without_changing_the_envelope() {
    async fn handler() -> ApiError {
        ApiError::rate_limited()
            .with_header(
                header::CACHE_CONTROL,
                HeaderValue::from_static("private, no-store"),
            )
            .with_retry_after(120)
            .with_header(X_REQUEST_ID, HeaderValue::from_static("error-request-id"))
    }

    let response = layers(
        Router::new().route("/limited", get(handler)),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .uri("/limited")
            .header(X_REQUEST_ID, "caller-request-id")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");

    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
    assert_eq!(response.headers()[header::RETRY_AFTER], "120");
    assert_eq!(response.headers()[X_REQUEST_ID], "caller-request-id");
    assert_eq!(
        response_json(response).await,
        json!({
            "error": {
                "code": "rate_limited",
                "message": "Too many requests",
                "request_id": "caller-request-id",
                "details": {}
            }
        })
    );
}

#[tokio::test]
async fn internal_cause_is_not_serialized() {
    let response =
        ApiError::internal(io::Error::other("database password secret-value")).into_response();
    let body = response_json(response).await;

    assert_eq!(body["error"]["code"], "internal");
    assert_eq!(body["error"]["message"], "An internal error occurred");
    assert!(!body.to_string().contains("database"));
    assert!(!body.to_string().contains("secret-value"));
}

#[tokio::test]
async fn panic_becomes_safe_500_envelope() {
    async fn handler() {
        panic!("sensitive panic message");
    }

    let response = layers(
        Router::new().route("/panic", get(handler)),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .uri("/panic")
            .header(X_REQUEST_ID, "panic-request")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "internal");
    assert_eq!(body["error"]["request_id"], "panic-request");
    assert!(!body.to_string().contains("sensitive"));
}

#[tokio::test]
async fn timeout_becomes_standard_envelope() {
    async fn handler() {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let options = HttpOptions {
        request_timeout: Duration::from_millis(1),
        ..HttpOptions::default()
    };
    let response = layers(Router::new().route("/slow", get(handler)), options)
        .oneshot(
            Request::builder()
                .uri("/slow")
                .header(X_REQUEST_ID, "timeout-request")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    assert_eq!(
        response_json(response).await,
        json!({
            "error": {
                "code": "request_timeout",
                "message": "The request timed out",
                "request_id": "timeout-request",
                "details": {}
            }
        })
    );
}

#[tokio::test]
async fn body_limit_becomes_standard_envelope() {
    let options = HttpOptions {
        body_size_limit: 3,
        ..HttpOptions::default()
    };
    let response = layers(Router::new().route("/upload", post(|| async {})), options)
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/upload")
                .header(X_REQUEST_ID, "large-request")
                .header(header::CONTENT_LENGTH, "4")
                .body(Body::from("four"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response_json(response).await,
        json!({
            "error": {
                "code": "payload_too_large",
                "message": "The request body is too large",
                "request_id": "large-request",
                "details": {}
            }
        })
    );
}

#[tokio::test]
async fn standardized_extractors_map_json_path_and_query_rejections() {
    #[derive(serde::Deserialize)]
    struct Input {
        count: usize,
    }

    async fn json_handler(ApiJson(input): ApiJson<Input>) -> String {
        input.count.to_string()
    }
    async fn path_handler(ApiPath(id): ApiPath<u64>) -> String {
        id.to_string()
    }
    async fn query_handler(ApiQuery(input): ApiQuery<Input>) -> String {
        input.count.to_string()
    }

    let app = finalize(
        Router::new()
            .route("/json", post(json_handler))
            .route("/path/{id}", get(path_handler))
            .route("/query", get(query_handler)),
        HttpOptions::default(),
    );
    for (request, detail) in [
        (
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .header(X_REQUEST_ID, "json-rejection")
                .body(Body::from("not-json"))
                .expect("request"),
            "body",
        ),
        (
            Request::builder()
                .uri("/path/not-a-number")
                .header(X_REQUEST_ID, "path-rejection")
                .body(Body::empty())
                .expect("request"),
            "path",
        ),
        (
            Request::builder()
                .uri("/query?count=not-a-number")
                .header(X_REQUEST_ID, "query-rejection")
                .body(Body::empty())
                .expect("request"),
            "query",
        ),
    ] {
        let response = app.clone().oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], "validation_failed");
        assert!(body["error"]["details"].get(detail).is_some());
        assert!(
            body["error"]["request_id"]
                .as_str()
                .is_some_and(|request_id| request_id.ends_with("-rejection"))
        );
    }
}

#[tokio::test]
async fn malformed_json_has_the_syntax_class() {
    let response = json_request(
        classified_json_options(),
        Some("application/json"),
        "not-json",
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "invalid_json");
    assert_eq!(
        body["error"]["details"],
        json!({"body": "must contain valid JSON"})
    );
}

#[tokio::test]
async fn missing_json_content_type_has_the_content_type_class() {
    let response = json_request(classified_json_options(), None, r#"{"count": 1}"#).await;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "unsupported_media_type");
    assert_eq!(body["error"]["details"], json!({}));
}

#[tokio::test]
async fn unsupported_json_content_type_has_the_content_type_class() {
    let response = json_request(
        classified_json_options(),
        Some("text/plain"),
        r#"{"count": 1}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "unsupported_media_type"
    );
}

#[tokio::test]
async fn json_field_type_mismatch_has_the_data_class() {
    let response = json_request(
        classified_json_options(),
        Some("application/json"),
        r#"{"count": "many"}"#,
    )
    .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "validation_failed");
    assert_eq!(
        body["error"]["details"],
        json!({"body": "must match the request schema"})
    );
}

#[tokio::test]
async fn configured_body_limit_has_the_body_too_large_class() {
    const BODY_LIMIT: usize = 16;
    let options = HttpOptions {
        body_size_limit: BODY_LIMIT,
        ..classified_json_options()
    };
    let body = r#"{"count": 123456789}"#;
    let app = finalize(
        Router::new().route("/json", post(json_input_handler)),
        options,
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::CONTENT_LENGTH, body.len())
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let body = response_json(response).await;
    assert_eq!(body["error"]["code"], "payload_too_large");
    assert_eq!(body["error"]["details"], json!({}));
}

#[tokio::test]
async fn route_body_limit_has_the_body_too_large_class() {
    const BODY_LIMIT: usize = 16;
    let body = r#"{"count": 123456789}"#;
    let app = finalize(
        Router::new().route(
            "/json",
            post(json_input_handler).layer(DefaultBodyLimit::max(BODY_LIMIT)),
        ),
        classified_json_options(),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response_json(response).await["error"]["code"],
        "payload_too_large"
    );
}

#[tokio::test]
async fn classified_json_rejection_propagates_the_request_id() {
    let app = finalize(
        Router::new().route("/json", post(json_input_handler)),
        classified_json_options(),
    );
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .header(X_REQUEST_ID, "json-class-request")
                .body(Body::from("not-json"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.headers()[X_REQUEST_ID], "json-class-request");
    assert_eq!(
        response_json(response).await["error"]["request_id"],
        "json-class-request"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn json_rejections_do_not_log_or_return_body_and_parser_details() {
    const PRIVATE_BODY: &str = r#"{"count": private-body-marker}"#;
    init_test_tracing();

    let response = json_request(
        classified_json_options(),
        Some("application/json"),
        PRIVATE_BODY,
    )
    .await;
    let response_body = response_json(response).await.to_string();
    let records = format!(
        "{:?}{:?}",
        CAPTURED_SPANS.lock().expect("span capture lock"),
        CAPTURED_EVENTS.lock().expect("event capture lock")
    );

    for private_text in ["private-body-marker", "line 1 column"] {
        assert!(!response_body.contains(private_text), "{response_body}");
        assert!(!records.contains(private_text), "{records}");
    }
}

#[tokio::test]
async fn query_collection_fields_accept_repeated_parameters() {
    #[derive(serde::Deserialize)]
    struct Filters {
        tag: Vec<String>,
    }

    async fn handler(Query(filters): Query<Filters>) -> String {
        filters.tag.join(",")
    }

    let response = Router::new()
        .route("/items", get(handler))
        .oneshot(
            Request::builder()
                .uri("/items?tag=rust&tag=typescript")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body"),
        "rust,typescript"
    );
}

#[test]
fn utoipa_infers_baukit_query_collections_as_query_parameters() {
    let document = InferredQueryApi::openapi();
    let json = serde_json::to_value(document).expect("OpenAPI JSON");
    let parameters = json["paths"]["/inferred-items"]["get"]["parameters"]
        .as_array()
        .unwrap_or_else(|| panic!("missing inferred parameters in {json}"));

    assert_eq!(parameters.len(), 1, "{json}");
    assert_eq!(parameters[0]["name"], "tag");
    assert_eq!(parameters[0]["in"], "query");
    assert_eq!(parameters[0]["schema"]["type"], "array");
    assert_ne!(parameters[0]["in"], "path");
}

#[tokio::test]
async fn validation_field_helpers_build_standard_details() {
    let one =
        response_json(ApiError::validation_field("limit", "must be at most 100").into_response())
            .await;
    assert_eq!(
        one["error"]["details"],
        json!({"limit": "must be at most 100"})
    );

    let many = response_json(
        ApiError::validation_fields([
            ("name", "must not be empty"),
            ("limit", "must be at least 1"),
        ])
        .into_response(),
    )
    .await;
    assert_eq!(
        many["error"]["details"],
        json!({"limit": "must be at least 1", "name": "must not be empty"})
    );
}

#[tokio::test]
async fn legacy_json_rejection_code_remains_compatible() {
    #[derive(serde::Deserialize)]
    struct Input {
        count: usize,
    }

    async fn json_handler(ApiJson(input): ApiJson<Input>) -> String {
        input.count.to_string()
    }
    async fn query_handler(ApiQuery(input): ApiQuery<Input>) -> String {
        input.count.to_string()
    }

    let options = HttpOptions::default()
        .with_json_rejection_code("invalid_json")
        .expect("valid error code");
    let app = finalize(
        Router::new()
            .route("/json", post(json_handler))
            .route("/query", get(query_handler)),
        options,
    );

    let json_response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/json")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from("not-json"))
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(json_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(json_response).await["error"]["code"],
        "invalid_json"
    );

    let query_response = app
        .oneshot(
            Request::builder()
                .uri("/query?count=not-a-number")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(query_response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response_json(query_response).await["error"]["code"],
        "validation_failed"
    );
}

#[test]
fn json_rejection_code_must_be_snake_case() {
    assert_eq!(
        HttpOptions::default()
            .with_json_rejection_code("Invalid JSON")
            .expect_err("invalid code"),
        HttpOptionsError::InvalidJsonRejectionCode("Invalid JSON".to_owned())
    );
}

#[test]
fn class_specific_json_rejection_codes_must_be_snake_case() {
    assert_eq!(
        JsonRejectionCodes::new(
            "payload_too_large",
            "unsupported_media_type",
            "Invalid JSON",
            "validation_failed",
        )
        .expect_err("invalid syntax code"),
        HttpOptionsError::InvalidJsonRejectionCode("Invalid JSON".to_owned())
    );
}

#[test]
fn default_json_rejection_codes_are_stable() {
    let codes = JsonRejectionCodes::default();

    assert_eq!(codes.body_too_large(), "payload_too_large");
    assert_eq!(codes.content_type(), "unsupported_media_type");
    assert_eq!(codes.syntax(), "invalid_json");
    assert_eq!(codes.data_shape(), "validation_failed");
    assert_eq!(
        HttpOptions::default()
            .with_json_rejection_codes(codes.clone())
            .json_rejection_codes(),
        Some(&codes)
    );
}

#[tokio::test]
async fn finalize_maps_unmatched_routes_and_methods() {
    let app = finalize(
        Router::new().route("/items", get(|| async {})),
        HttpOptions::default(),
    );

    for (method, uri, status, code) in [
        (Method::GET, "/missing", StatusCode::NOT_FOUND, "not_found"),
        (
            Method::POST,
            "/items",
            StatusCode::METHOD_NOT_ALLOWED,
            "method_not_allowed",
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(X_REQUEST_ID, "routing-rejection")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), status);
        let body = response_json(response).await;
        assert_eq!(body["error"]["code"], code);
        assert_eq!(body["error"]["request_id"], "routing-rejection");
    }
}

#[tokio::test]
async fn cors_preflight_uses_explicit_origin() {
    let options = HttpOptions::default()
        .with_allowed_origins(["https://app.example.com"])
        .expect("valid origin");
    let response = layers(Router::new().route("/items", post(|| async {})), options)
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/items")
                .header(header::ORIGIN, "https://app.example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "https://app.example.com"
    );
}

#[tokio::test]
async fn cors_preflight_adds_product_headers_to_the_defaults() {
    let options = HttpOptions::default()
        .with_allowed_origins(["https://app.example.com"])
        .expect("valid origin")
        .with_additional_allowed_headers(["Accept", "x-webhook-secret"])
        .expect("valid request headers");
    let response = layers(Router::new().route("/items", post(|| async {})), options)
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/items")
                .header(header::ORIGIN, "https://app.example.com")
                .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(
                    header::ACCESS_CONTROL_REQUEST_HEADERS,
                    "accept,content-type,x-webhook-secret",
                )
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let allowed = response.headers()[header::ACCESS_CONTROL_ALLOW_HEADERS]
        .to_str()
        .expect("allowed headers")
        .split(',')
        .map(str::trim)
        .collect::<Vec<_>>();
    for expected in ["accept", "content-type", "x-webhook-secret"] {
        assert!(
            allowed.iter().any(|header| header == &expected),
            "missing {expected} from {allowed:?}"
        );
    }
}

#[test]
fn additional_cors_headers_are_validated_and_deduplicated() {
    let options = HttpOptions::default()
        .with_additional_allowed_headers(["Accept", "accept"])
        .expect("valid headers");
    assert_eq!(options.additional_allowed_headers().len(), 1);
    assert_eq!(
        HttpOptions::default()
            .with_additional_allowed_headers(["not a header"])
            .expect_err("invalid header"),
        HttpOptionsError::InvalidHeaderName("not a header".to_owned())
    );
}

#[tokio::test(flavor = "current_thread")]
async fn metrics_use_bounded_template_labels_and_raw_status() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = metrics::set_default_local_recorder(&recorder);
    let custom_method = Method::from_bytes(b"BREW").expect("custom method");
    let response = layers(
        Router::new().route("/widgets/{id}", any(|| async { StatusCode::IM_A_TEAPOT })),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .method(custom_method)
            .uri("/widgets/private-user-42")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::IM_A_TEAPOT);

    let metrics = snapshotter.snapshot().into_vec();
    let counter = find_metric(&metrics, HTTP_REQUESTS_TOTAL);
    assert_eq!(metric_labels(counter.0).get("method"), Some(&"OTHER"));
    assert_eq!(
        metric_labels(counter.0).get("route"),
        Some(&"/widgets/{id}")
    );
    assert_eq!(metric_labels(counter.0).get("status"), Some(&"418"));
    assert!(!format!("{:?}", counter.0).contains("private-user-42"));
    assert_eq!(counter.1, &DebugValue::Counter(1));

    let histogram = find_metric(&metrics, HTTP_REQUEST_DURATION_SECONDS);
    assert_eq!(metric_labels(histogram.0).get("status"), Some(&"418"));
    assert!(matches!(histogram.1, DebugValue::Histogram(values) if values.len() == 1));

    let gauge = find_metric(&metrics, HTTP_REQUESTS_IN_FLIGHT);
    assert_eq!(gauge.1, &DebugValue::Gauge(0.0.into()));
}

#[tokio::test(flavor = "current_thread")]
async fn unmatched_requests_use_unmatched_route_label() {
    let recorder = DebuggingRecorder::new();
    let snapshotter = recorder.snapshotter();
    let _recorder_guard = metrics::set_default_local_recorder(&recorder);
    let response = finalize(Router::new(), HttpOptions::default())
        .oneshot(
            Request::builder()
                .uri("/not/a/real/route")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let metrics = snapshotter.snapshot().into_vec();
    let counter = find_metric(&metrics, HTTP_REQUESTS_TOTAL);
    assert_eq!(metric_labels(counter.0).get("route"), Some(&"unmatched"));
    assert_eq!(metric_labels(counter.0).get("status"), Some(&"404"));
    assert!(!format!("{:?}", counter.0).contains("not/a/real/route"));
}

#[tokio::test(flavor = "current_thread")]
async fn span_uses_template_and_propagates_w3c_parent() {
    const TRACE_ID: &str = "0af7651916cd43dd8448eb211c80319c";
    init_test_tracing();

    let response = layers(
        Router::new().route("/users/{id}", get(|| async {})),
        HttpOptions::default(),
    )
    .oneshot(
        Request::builder()
            .uri("/users/private-user-42")
            .header("traceparent", format!("00-{TRACE_ID}-b7ad6b7169203331-01"))
            .header("tracestate", "vendor=value")
            .body(Body::empty())
            .expect("request"),
    )
    .await
    .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let traceparent = response.headers()["traceparent"]
        .to_str()
        .expect("traceparent response header");
    assert!(traceparent.starts_with(&format!("00-{TRACE_ID}-")));
    assert_eq!(response.headers()["tracestate"], "vendor=value");

    let spans = CAPTURED_SPANS.lock().expect("span capture lock");
    let request_span = spans
        .iter()
        .find(|span| {
            span.get("http.route")
                .is_some_and(|route| route == "/users/{id}")
        })
        .unwrap_or_else(|| panic!("request span missing from {spans:?}"));
    assert_eq!(
        request_span.get("otel.name"),
        Some(&"GET /users/{id}".to_owned())
    );
    assert_eq!(
        request_span.get("http.route"),
        Some(&"/users/{id}".to_owned())
    );
    assert!(!format!("{request_span:?}").contains("private-user-42"));
}

fn find_metric<'a>(
    metrics: &'a [(
        CompositeKey,
        Option<metrics::Unit>,
        Option<metrics::SharedString>,
        DebugValue,
    )],
    name: &str,
) -> (&'a CompositeKey, &'a DebugValue) {
    metrics
        .iter()
        .find(|(key, _, _, _)| key.key().name() == name)
        .map(|(key, _, _, value)| (key, value))
        .expect("metric present")
}

fn metric_labels(key: &CompositeKey) -> HashMap<&str, &str> {
    key.key()
        .labels()
        .map(|label| (label.key(), label.value()))
        .collect()
}

type CapturedSpans = Arc<Mutex<Vec<BTreeMap<String, String>>>>;

struct SpanCapture(CapturedSpans);

static CAPTURED_SPANS: LazyLock<CapturedSpans> = LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
static CAPTURED_EVENTS: LazyLock<CapturedSpans> =
    LazyLock::new(|| Arc::new(Mutex::new(Vec::new())));
static TEST_TRACING: Once = Once::new();

fn init_test_tracing() {
    TEST_TRACING.call_once(|| {
        global::set_text_map_propagator(TraceContextPropagator::new());
        let provider = SdkTracerProvider::builder().build();
        let tracer = provider.tracer("baukit-http-test");
        let subscriber = Registry::default()
            .with(SpanCapture(Arc::clone(&CAPTURED_SPANS)))
            .with(tracing_opentelemetry::layer().with_tracer(tracer));
        tracing::subscriber::set_global_default(subscriber).expect("set test tracing subscriber");
    });
}

impl<S> Layer<S> for SpanCapture
where
    S: Subscriber,
{
    fn on_new_span(
        &self,
        attributes: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields =
            BTreeMap::from([("span".to_owned(), attributes.metadata().name().to_owned())]);
        attributes.record(&mut FieldCapture(&mut fields));
        self.0.lock().expect("span capture lock").push(fields);
    }

    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _context: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut fields = BTreeMap::from([("event".to_owned(), event.metadata().name().to_owned())]);
        event.record(&mut FieldCapture(&mut fields));
        CAPTURED_EVENTS
            .lock()
            .expect("event capture lock")
            .push(fields);
    }
}

struct FieldCapture<'a>(&'a mut BTreeMap<String, String>);

impl Visit for FieldCapture<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.0.insert(field.name().to_owned(), format!("{value:?}"));
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.0.insert(field.name().to_owned(), value.to_owned());
    }
}
