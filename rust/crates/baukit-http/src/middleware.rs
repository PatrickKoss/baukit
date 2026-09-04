use std::{any::Any, fmt, time::Instant};

use axum::{
    Router,
    body::Body,
    extract::{FromRequestParts, MatchedPath, Request, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, Method, StatusCode,
        header::{AUTHORIZATION, CONTENT_TYPE},
        request::Parts,
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use metrics::Gauge;
use opentelemetry::{
    Context, global,
    propagation::{Extractor, Injector},
};
use tokio::time;
use tower::limit::ConcurrencyLimitLayer;
use tower_http::{
    catch_panic::CatchPanicLayer,
    cors::{AllowOrigin, CorsLayer},
    limit::RequestBodyLimitLayer,
};
use tracing::Instrument as _;
use tracing_opentelemetry::OpenTelemetrySpanExt as _;
use uuid::Uuid;

use crate::{ApiError, HttpOptions};

/// The standard request ID header.
pub const X_REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

const TRACEPARENT: HeaderName = HeaderName::from_static("traceparent");
const TRACESTATE: HeaderName = HeaderName::from_static("tracestate");
const UNMATCHED_ROUTE: &str = "unmatched";

/// Counter metric for completed HTTP requests, labeled `method`, `route`, and `status`.
pub const HTTP_REQUESTS_TOTAL: &str = "http_requests_total";
/// Histogram metric for HTTP request seconds, labeled `method`, `route`, and `status`.
pub const HTTP_REQUEST_DURATION_SECONDS: &str = "http_request_duration_seconds";
/// Gauge metric for active HTTP requests, labeled `method` and `route`.
pub const HTTP_REQUESTS_IN_FLIGHT: &str = "http_requests_in_flight";

/// Required Prometheus buckets for [`HTTP_REQUEST_DURATION_SECONDS`], in seconds.
///
/// This crate records through the metrics facade and deliberately does not own a
/// recorder. The recorder setup (normally `baukit-telemetry`) must pass this
/// slice to `metrics_exporter_prometheus::PrometheusBuilder::set_buckets` or a
/// metric-specific equivalent before installing the recorder.
pub const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

tokio::task_local! {
    static CURRENT_REQUEST_ID: RequestId;
}

/// A request identifier accepted from the client or generated as a UUID.
///
/// The defaults stack inserts this value into request extensions, so handlers
/// can use it directly as an Axum extractor.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct RequestId(String);

impl RequestId {
    fn from_headers(headers: &HeaderMap) -> Self {
        headers
            .get(&X_REQUEST_ID)
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map_or_else(Self::generate, |value| Self(value.to_owned()))
    }

    fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Returns the request identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn header_value(
        &self,
    ) -> Result<HeaderValue, axum::http::header::InvalidHeaderValue> {
        HeaderValue::from_str(&self.0)
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl<S> FromRequestParts<S> for RequestId
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts.extensions.get::<Self>().cloned().ok_or_else(|| {
            ApiError::internal(std::io::Error::other(
                "RequestId extractor used without baukit_http::layers",
            ))
        })
    }
}

/// Extracts W3C trace context from request headers using the global propagator.
#[must_use]
pub fn extract_trace_context(headers: &HeaderMap) -> Context {
    global::get_text_map_propagator(|propagator| propagator.extract(&HeaderExtractor(headers)))
}

/// Injects `context` into outbound headers using the global W3C propagator.
///
/// Use this for outbound HTTP requests. The response path uses the same helper
/// to expose applicable trace headers to callers.
pub fn inject_trace_context(context: &Context, headers: &mut HeaderMap) {
    global::get_text_map_propagator(|propagator| {
        propagator.inject_context(context, &mut HeaderInjector(headers));
    });
}

/// Injects the current tracing span's OpenTelemetry context into outbound headers.
pub fn inject_current_trace_context(headers: &mut HeaderMap) {
    inject_trace_context(&tracing::Span::current().context(), headers);
}

/// Applies the complete Baukit HTTP defaults stack to an Axum router.
///
/// Construct `options` through [`HttpOptions::default`] or
/// [`HttpOptions::from_config`]. Invalid zero-valued public limits are replaced
/// by their sane defaults; validated constructors reject them earlier.
pub fn layers<S>(router: Router<S>, options: HttpOptions) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let options = options.validate().unwrap_or_default();
    let cors = cors_layer(&options);
    let body_size_limit = options.body_size_limit;
    let concurrency_limit = options.concurrency_limit;

    router
        .layer(RequestBodyLimitLayer::new(body_size_limit))
        .layer(ConcurrencyLimitLayer::new(concurrency_limit))
        .layer(cors)
        .layer(CatchPanicLayer::custom(panic_response))
        .layer(middleware::from_fn_with_state(options, request_lifecycle))
}

fn cors_layer(options: &HttpOptions) -> CorsLayer {
    let mut allowed_headers = vec![
        AUTHORIZATION,
        CONTENT_TYPE,
        X_REQUEST_ID,
        TRACEPARENT,
        TRACESTATE,
    ];
    for header in &options.additional_allowed_headers {
        if !allowed_headers.contains(header) {
            allowed_headers.push(header.clone());
        }
    }
    let mut cors = CorsLayer::new()
        .allow_methods([
            Method::GET,
            Method::HEAD,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers(allowed_headers)
        .expose_headers([X_REQUEST_ID, TRACEPARENT, TRACESTATE]);
    if !options.allowed_origins.is_empty() {
        cors = cors.allow_origin(AllowOrigin::list(options.allowed_origins.clone()));
    }
    if options.allow_credentials {
        cors = cors.allow_credentials(true);
    }
    cors
}

async fn request_lifecycle(
    State(options): State<HttpOptions>,
    mut request: Request,
    next: Next,
) -> Response {
    let request_id = RequestId::from_headers(request.headers());
    request.extensions_mut().insert(request_id.clone());
    request
        .extensions_mut()
        .insert(options.json_rejection_mode.clone());
    CURRENT_REQUEST_ID
        .scope(request_id, lifecycle_inner(options, request, next))
        .await
}

async fn lifecycle_inner(options: HttpOptions, request: Request, next: Next) -> Response {
    let request_id = current_request_id();
    let method = bounded_method(request.method());
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or(UNMATCHED_ROUTE, MatchedPath::as_str)
        .to_owned();
    let parent_context = extract_trace_context(request.headers());
    let span_name = format!("{method} {route}");
    let span = tracing::info_span!(
        "http.request",
        otel.name = %span_name,
        http.request.method = method,
        http.route = %route,
        http.response.status_code = tracing::field::Empty,
        request_id = %request_id,
    );
    if let Err(error) = span.set_parent(parent_context) {
        tracing::debug!(error = %error, "could not attach inbound trace context");
    }

    async move {
        let started = Instant::now();
        let in_flight = metrics::gauge!(
            HTTP_REQUESTS_IN_FLIGHT,
            "method" => method,
            "route" => route.clone()
        );
        in_flight.increment(1.0);
        let _in_flight = InFlightGuard(in_flight);

        let mut response = match time::timeout(options.request_timeout, next.run(request)).await {
            Ok(mut response) => {
                if response.extensions_mut().remove::<PanicCaught>().is_some() {
                    tracing::error!("request handler panicked");
                    ApiError::internal_without_cause().into_response()
                } else if response.status() == StatusCode::PAYLOAD_TOO_LARGE {
                    ApiError::configured_payload_too_large(&options.json_rejection_mode)
                        .into_response()
                } else {
                    response
                }
            }
            Err(_) => {
                tracing::warn!("request timed out");
                ApiError::timeout().into_response()
            }
        };

        let status = response.status();
        let status_label = status.as_u16().to_string();
        tracing::Span::current().record("http.response.status_code", status.as_u16());
        metrics::counter!(
            HTTP_REQUESTS_TOTAL,
            "method" => method,
            "route" => route.clone(),
            "status" => status_label.clone()
        )
        .increment(1);
        metrics::histogram!(
            HTTP_REQUEST_DURATION_SECONDS,
            "method" => method,
            "route" => route,
            "status" => status_label
        )
        .record(started.elapsed().as_secs_f64());

        response.headers_mut().insert(
            X_REQUEST_ID,
            current_request_id()
                .header_value()
                .expect("request IDs originate from a valid header or UUID"),
        );
        inject_current_trace_context(response.headers_mut());
        response
    }
    .instrument(span)
    .await
}

pub(crate) fn current_request_id() -> RequestId {
    CURRENT_REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| RequestId::generate())
}

fn bounded_method(method: &Method) -> &'static str {
    match *method {
        Method::CONNECT => "CONNECT",
        Method::DELETE => "DELETE",
        Method::GET => "GET",
        Method::HEAD => "HEAD",
        Method::OPTIONS => "OPTIONS",
        Method::PATCH => "PATCH",
        Method::POST => "POST",
        Method::PUT => "PUT",
        Method::TRACE => "TRACE",
        _ => "OTHER",
    }
}

#[derive(Clone, Copy, Debug)]
struct PanicCaught;

fn panic_response(_panic: Box<dyn Any + Send + 'static>) -> Response<Body> {
    let mut response = StatusCode::INTERNAL_SERVER_ERROR.into_response();
    response.extensions_mut().insert(PanicCaught);
    response
}

struct InFlightGuard(Gauge);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.decrement(1.0);
    }
}

struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|value| value.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(HeaderName::as_str).collect()
    }
}

struct HeaderInjector<'a>(&'a mut HeaderMap);

impl Injector for HeaderInjector<'_> {
    fn set(&mut self, key: &str, value: String) {
        if let (Ok(name), Ok(value)) = (HeaderName::from_bytes(key.as_bytes()), value.parse()) {
            self.0.insert(name, value);
        }
    }
}
