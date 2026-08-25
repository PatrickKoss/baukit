use std::{error::Error, sync::Arc};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Method, Request, StatusCode, header},
};
use baukit_config::HttpConfig;
use baukit_ops::TrafficGate;
use baukit_runtime::{DeploymentEnvironment, ProcessKind, ServiceInfo, build_info};
use baukit_telemetry::TelemetryBuilder;
use minimal_api::{AppState, api_router, openapi_document, ops_router};
use serde_json::{Value, json};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn Error>>;

#[tokio::test]
async fn service_contract() -> TestResult {
    let service = ServiceInfo::new(
        "minimal-api",
        ProcessKind::Api,
        build_info!(),
        DeploymentEnvironment::Local,
    );
    let telemetry = Arc::new(
        TelemetryBuilder::new(service.telemetry_identity().clone())
            .filter("off")
            .init()?,
    );
    let state = AppState::new(10);
    let traffic_gate = TrafficGate::new();
    let api = api_router(state.clone(), &HttpConfig::default())?;
    let operations = ops_router(
        state,
        service.telemetry_identity().clone(),
        telemetry.prometheus_handle().clone(),
        traffic_gate.clone(),
    )?;

    let (status, headers, body) = call(
        &api,
        json_request(
            Method::POST,
            "/notes",
            json!({"title": "First", "body": "A small note"}),
            Some("request-create-1"),
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(headers["x-request-id"], "request-create-1");
    assert_eq!(json_body(&body)?["id"], 1);

    let (status, _, body) = call(&api, empty_request(Method::GET, "/notes/1")?).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)?["title"], "First");

    let (status, _, body) = call(&api, empty_request(Method::GET, "/notes")?).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)?.as_array().map(Vec::len), Some(1));

    let (status, headers, body) = call(
        &api,
        json_request(
            Method::POST,
            "/notes",
            json!({"title": "  ", "body": "invalid"}),
            Some("request-validation-1"),
        )?,
    )
    .await?;
    let validation = json_body(&body)?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(headers["x-request-id"], "request-validation-1");
    assert_eq!(validation["error"]["code"], "validation_failed");
    assert_eq!(validation["error"]["message"], "The request is invalid");
    assert_eq!(validation["error"]["request_id"], "request-validation-1");
    assert_eq!(validation["error"]["details"]["title"], "must not be empty");

    let (status, _, body) = call(
        &api,
        Request::builder()
            .method(Method::POST)
            .uri("/notes")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("not-json"))?,
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(json_body(&body)?["error"]["code"], "validation_failed");

    let (status, headers, body) = call(
        &api,
        empty_request_with_id(Method::GET, "/notes/404", "request-missing-1")?,
    )
    .await?;
    let missing = json_body(&body)?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers["x-request-id"], "request-missing-1");
    assert_eq!(missing["error"]["code"], "not_found");
    assert_eq!(missing["error"]["request_id"], "request-missing-1");
    assert_eq!(missing["error"]["details"], json!({}));

    let (status, _, body) = call(
        &api,
        empty_request_with_id(Method::GET, "/missing-route", "request-route-1")?,
    )
    .await?;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(json_body(&body)?["error"]["request_id"], "request-route-1");

    let (status, _, body) = call(
        &api,
        empty_request_with_id(Method::GET, "/fail", "request-fail-1")?,
    )
    .await?;
    let deliberate_failure = json_body(&body)?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        deliberate_failure["error"]["details"]["example"],
        "this endpoint always fails"
    );

    let (status, _, body) = call(&operations, empty_request(Method::GET, "/healthz")?).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json_body(&body)?, json!({"status": "ok"}));

    let (status, _, body) = call(&operations, empty_request(Method::GET, "/readyz")?).await?;
    assert_eq!(status, StatusCode::OK);
    let readiness = json_body(&body)?;
    assert_eq!(readiness["status"], "ready");
    assert_eq!(readiness["accepting_traffic"], true);
    assert_eq!(readiness["checks"][0]["name"], "accepting_traffic");
    assert_eq!(readiness["checks"][1]["name"], "state");

    let (status, headers, body) =
        call(&operations, empty_request(Method::GET, "/metrics")?).await?;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers[header::CONTENT_TYPE]
            .to_str()?
            .starts_with("text/plain")
    );
    let metrics = String::from_utf8(body)?;
    for metric in [
        "http_requests_total",
        "http_request_duration_seconds",
        "http_requests_in_flight",
        "build_info",
    ] {
        assert!(metrics.contains(metric), "missing {metric} in:\n{metrics}");
    }
    assert!(!metrics.contains("http_requests_duration_seconds"));
    assert!(metrics.contains("route=\"/notes/{id}\""));
    assert!(metrics.contains("status=\"404\""));

    traffic_gate.stop_accepting();
    let (status, _, body) = call(&operations, empty_request(Method::GET, "/readyz")?).await?;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(&body)?["status"], "not_ready");

    telemetry.shutdown()?;
    Ok(())
}

#[test]
fn committed_openapi_has_no_drift() {
    baukit_openapi::assert_no_drift(
        &openapi_document(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/openapi.json"),
    );
}

async fn call(
    app: &Router,
    request: Request<Body>,
) -> Result<(StatusCode, HeaderMap, Vec<u8>), Box<dyn Error>> {
    let response = app.clone().oneshot(request).await?;
    let (parts, body) = response.into_parts();
    let body = to_bytes(body, usize::MAX).await?.to_vec();
    Ok((parts.status, parts.headers, body))
}

fn json_request(
    method: Method,
    uri: &str,
    body: Value,
    request_id: Option<&str>,
) -> Result<Request<Body>, Box<dyn Error>> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(request_id) = request_id {
        builder = builder.header("x-request-id", request_id);
    }
    Ok(builder.body(Body::from(serde_json::to_vec(&body)?))?)
}

fn empty_request(method: Method, uri: &str) -> Result<Request<Body>, Box<dyn Error>> {
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())?)
}

fn empty_request_with_id(
    method: Method,
    uri: &str,
    request_id: &str,
) -> Result<Request<Body>, Box<dyn Error>> {
    Ok(Request::builder()
        .method(method)
        .uri(uri)
        .header("x-request-id", request_id)
        .body(Body::empty())?)
}

fn json_body(body: &[u8]) -> Result<Value, serde_json::Error> {
    serde_json::from_slice(body)
}
