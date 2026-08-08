use std::sync::Arc;
use std::time::Duration;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use baukit_telemetry::{
    DeploymentEnvironment, ProcessKind,
    metrics::{Key, Level, Metadata, Recorder},
};
use metrics_exporter_prometheus::PrometheusBuilder;
use serde_json::Value;
use tower::ServiceExt as _;

use super::*;

fn identity() -> ServiceIdentity {
    ServiceIdentity::new(
        "orders",
        ProcessKind::Api,
        "1.2.3",
        "abc1234",
        DeploymentEnvironment::Local,
    )
}

fn test_metrics() -> PrometheusHandle {
    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    recorder
        .register_counter(
            &Key::from_name("ops_test_total"),
            &Metadata::new("baukit_ops_test", Level::INFO, Some(module_path!())),
        )
        .increment(7);
    handle
}

fn app(readiness: ReadinessRegistry, traffic_gate: TrafficGate) -> Router {
    OpsRouter::new(identity(), test_metrics())
        .with_readiness(readiness)
        .with_traffic_gate(traffic_gate)
        .into_router()
}

async fn get(app: Router, uri: &str) -> axum::response::Response {
    app.oneshot(
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("valid request"),
    )
    .await
    .expect("ops router response")
}

async fn json_body(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("valid JSON response")
}

#[tokio::test]
async fn healthz_is_always_live() {
    let response = get(
        app(ReadinessRegistry::new(), TrafficGate::new()),
        "/healthz",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        serde_json::json!({ "status": "ok" })
    );
}

#[tokio::test]
async fn readyz_returns_ok_when_all_checks_pass() {
    let readiness = ReadinessRegistry::new();
    readiness
        .register_fn_default("database", || async { Ok(()) })
        .expect("unique check");
    readiness
        .register_fn_default("queue", || async { Ok(()) })
        .expect("unique check");

    let response = get(app(readiness, TrafficGate::new()), "/readyz").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["status"], "ready");
    assert_eq!(body["accepting_traffic"], true);
    assert_eq!(body["checks"][0]["name"], "accepting_traffic");
    assert_eq!(body["checks"][1]["name"], "database");
    assert_eq!(body["checks"][2]["name"], "queue");
    assert!(
        body["checks"]
            .as_array()
            .expect("checks array")
            .iter()
            .all(|check| check["status"] == "pass")
    );
}

#[tokio::test]
async fn readyz_runs_checks_concurrently() {
    let readiness = ReadinessRegistry::new();
    let rendezvous = Arc::new(tokio::sync::Barrier::new(2));
    for name in ["database", "queue"] {
        let rendezvous = Arc::clone(&rendezvous);
        readiness
            .register_fn(name, Duration::from_millis(100), move || {
                let rendezvous = Arc::clone(&rendezvous);
                async move {
                    rendezvous.wait().await;
                    Ok(())
                }
            })
            .expect("unique check");
    }

    let response = get(app(readiness, TrafficGate::new()), "/readyz").await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn readyz_reports_each_failure_and_sanitizes_its_error() {
    let readiness = ReadinessRegistry::new();
    readiness
        .register_fn_default("database", || async {
            Err(ReadinessError::new(" database\n unavailable\t "))
        })
        .expect("unique check");
    readiness
        .register_fn_default("queue", || async { Ok(()) })
        .expect("unique check");

    let response = get(app(readiness, TrafficGate::new()), "/readyz").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["status"], "not_ready");
    assert_eq!(body["checks"][1]["name"], "database");
    assert_eq!(body["checks"][1]["status"], "fail");
    assert_eq!(body["checks"][1]["error"], "database unavailable");
    assert_eq!(body["checks"][2]["status"], "pass");
}

#[tokio::test]
async fn readyz_times_out_an_individual_check() {
    let readiness = ReadinessRegistry::new();
    readiness
        .register_fn("database", Duration::from_millis(10), || async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(())
        })
        .expect("unique check");

    let response = get(app(readiness, TrafficGate::new()), "/readyz").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["checks"][1]["status"], "timed_out");
    assert_eq!(body["checks"][1]["error"], "timed out after 10 ms");
}

#[tokio::test]
async fn traffic_gate_flip_fails_readiness_before_shutdown() {
    let traffic_gate = TrafficGate::new();
    let router = app(ReadinessRegistry::new(), traffic_gate.clone());
    assert_eq!(
        get(router.clone(), "/readyz").await.status(),
        StatusCode::OK
    );

    traffic_gate.stop_accepting();
    let response = get(router, "/readyz").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = json_body(response).await;
    assert_eq!(body["accepting_traffic"], false);
    assert_eq!(body["checks"][0]["status"], "fail");
    assert_eq!(body["checks"][0]["error"], "service is draining");
}

#[tokio::test]
async fn metrics_renders_the_injected_prometheus_handle() {
    let response = get(
        app(ReadinessRegistry::new(), TrafficGate::new()),
        "/metrics",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        PROMETHEUS_CONTENT_TYPE
    );
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read response body");
    let body = String::from_utf8(bytes.to_vec()).expect("metrics are UTF-8");
    assert!(body.contains("# TYPE ops_test_total counter"), "{body}");
    assert!(body.contains("ops_test_total 7"), "{body}");
}

#[tokio::test]
async fn buildinfo_uses_telemetry_service_identity() {
    let response = get(
        app(ReadinessRegistry::new(), TrafficGate::new()),
        "/buildinfo",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    assert_eq!(body["service_name"], "orders-api");
    assert_eq!(body["version"], "1.2.3");
    assert_eq!(body["commit"], "abc1234");
    assert_eq!(body["rust_version"], RUST_VERSION);
}

#[test]
fn registry_rejects_ambiguous_registrations() {
    let readiness = ReadinessRegistry::new();
    readiness
        .register_fn_default("database", || async { Ok(()) })
        .expect("first registration succeeds");
    assert_eq!(
        readiness
            .register_fn_default("database", || async { Ok(()) })
            .expect_err("duplicate name"),
        RegistrationError::DuplicateName("database".to_owned())
    );
    assert_eq!(
        readiness
            .register_fn_default("accepting_traffic", || async { Ok(()) })
            .expect_err("reserved name"),
        RegistrationError::ReservedName("accepting_traffic".to_owned())
    );
    assert_eq!(
        readiness
            .register_fn("queue", Duration::ZERO, || async { Ok(()) })
            .expect_err("zero timeout"),
        RegistrationError::ZeroTimeout("queue".to_owned())
    );
}

#[test]
fn readiness_error_is_utf8_safe_and_bounded() {
    let message = format!("{}\0é", "a".repeat(MAX_ERROR_LENGTH - 1));
    let error = ReadinessError::new(message);
    assert!(error.message().len() <= MAX_ERROR_LENGTH);
    assert!(error.message().is_char_boundary(error.message().len()));
    assert!(!error.message().chars().any(char::is_control));
}
