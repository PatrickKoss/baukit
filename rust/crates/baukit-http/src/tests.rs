use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{Arc, LazyLock, Mutex, Once},
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode, header},
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
async fn json_rejection_code_can_be_configured_without_changing_other_extractors() {
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
