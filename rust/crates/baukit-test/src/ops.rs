use std::fmt;

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, Response, StatusCode, header},
};
use baukit_ops::{ReadinessError, ReadinessRegistry};
use serde_json::Value;
use tower::ServiceExt as _;

const BODY_LIMIT: usize = 1024 * 1024;
const FAILURE_CHECK_NAME: &str = "baukit_test_forced_failure";

/// Violations found while exercising Baukit operations endpoints.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpsConformanceError {
    violations: Vec<String>,
}

impl OpsConformanceError {
    /// Returns violations in endpoint-check order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for OpsConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "operations conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for OpsConformanceError {}

/// Checks `/healthz`, both readiness states, and `/metrics` over HTTP.
///
/// The service must already be listening and must have been built from a clone
/// of `readiness`. The helper verifies passing readiness, permanently registers
/// a deliberate failure through that shared registry, and then verifies the
/// `503` per-check response. Use a dedicated service fixture for each call.
pub async fn check_ops_base_url_conformance(
    base_url: &str,
    readiness: &ReadinessRegistry,
) -> Result<(), OpsConformanceError> {
    let client = reqwest::Client::new();
    let base_url = base_url.trim_end_matches('/');
    let mut violations = Vec::new();

    match client.get(format!("{base_url}/healthz")).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            check_health(status, &body, &mut violations);
        }
        Err(error) => violations.push(format!("GET /healthz failed: {error}")),
    }
    match client.get(format!("{base_url}/readyz")).send().await {
        Ok(response) => {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            check_ready(status, &body, true, None, &mut violations);
        }
        Err(error) => violations.push(format!("GET /readyz failed: {error}")),
    }
    match client.get(format!("{base_url}/metrics")).send().await {
        Ok(response) => {
            let status = response.status();
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response.text().await.unwrap_or_default();
            check_metrics(status, content_type.as_deref(), &body, &mut violations);
        }
        Err(error) => violations.push(format!("GET /metrics failed: {error}")),
    }

    if register_failure(readiness, &mut violations) {
        match client.get(format!("{base_url}/readyz")).send().await {
            Ok(response) => {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                check_ready(
                    status,
                    &body,
                    false,
                    Some(FAILURE_CHECK_NAME),
                    &mut violations,
                );
            }
            Err(error) => violations.push(format!("GET failing /readyz failed: {error}")),
        }
    }

    finish(violations)
}

/// Panics when network operations endpoints violate their contract.
///
/// # Panics
///
/// Panics with all endpoint violations or transport failures.
pub async fn assert_ops_base_url_conformance(base_url: &str, readiness: &ReadinessRegistry) {
    if let Err(error) = check_ops_base_url_conformance(base_url, readiness).await {
        panic!("{error}");
    }
}

/// Checks operations endpoints in process through Tower `oneshot` requests.
///
/// `router` must have been built from a clone of `readiness`. The helper first
/// verifies passing readiness, then permanently registers a test failure in
/// that shared registry and verifies a `503` response containing a per-check
/// JSON result. Use a fresh registry/router fixture for each call.
pub async fn check_ops_router_conformance(
    router: &Router,
    readiness: &ReadinessRegistry,
) -> Result<(), OpsConformanceError> {
    let mut violations = Vec::new();

    match router_request(router, "/healthz").await {
        Ok(response) => {
            let (status, _, body) = response_parts(response).await;
            check_health(status, &body, &mut violations);
        }
        Err(error) => violations.push(error),
    }
    match router_request(router, "/readyz").await {
        Ok(response) => {
            let (status, _, body) = response_parts(response).await;
            check_ready(status, &body, true, None, &mut violations);
        }
        Err(error) => violations.push(error),
    }
    match router_request(router, "/metrics").await {
        Ok(response) => {
            let (status, content_type, body) = response_parts(response).await;
            check_metrics(status, content_type.as_deref(), &body, &mut violations);
        }
        Err(error) => violations.push(error),
    }

    if register_failure(readiness, &mut violations) {
        match router_request(router, "/readyz").await {
            Ok(response) => {
                let (status, _, body) = response_parts(response).await;
                check_ready(
                    status,
                    &body,
                    false,
                    Some(FAILURE_CHECK_NAME),
                    &mut violations,
                );
            }
            Err(error) => violations.push(error),
        }
    }

    finish(violations)
}

/// Panics when an in-process operations router violates its contract.
///
/// # Panics
///
/// Panics with all endpoint violations. The supplied registry is mutated as
/// described by [`check_ops_router_conformance`].
pub async fn assert_ops_router_conformance(router: &Router, readiness: &ReadinessRegistry) {
    if let Err(error) = check_ops_router_conformance(router, readiness).await {
        panic!("{error}");
    }
}

fn register_failure(readiness: &ReadinessRegistry, violations: &mut Vec<String>) -> bool {
    if let Err(error) = readiness.register_fn_default(FAILURE_CHECK_NAME, || async {
        Err(ReadinessError::new("forced test failure"))
    }) {
        violations.push(format!(
            "could not register `{FAILURE_CHECK_NAME}` readiness check: {error}"
        ));
        false
    } else {
        true
    }
}

async fn router_request(router: &Router, path: &str) -> Result<Response<Body>, String> {
    let request = Request::builder()
        .uri(path)
        .body(Body::empty())
        .map_err(|error| format!("could not build GET {path} request: {error}"))?;
    router
        .clone()
        .oneshot(request)
        .await
        .map_err(|error| format!("GET {path} failed: {error}"))
}

async fn response_parts(response: Response<Body>) -> (StatusCode, Option<String>, String) {
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let body = to_bytes(response.into_body(), BODY_LIMIT)
        .await
        .map_or_else(
            |error| format!("<could not read response body: {error}>"),
            |bytes| String::from_utf8_lossy(&bytes).into_owned(),
        );
    (status, content_type, body)
}

fn check_health(status: StatusCode, body: &str, violations: &mut Vec<String>) {
    if status != StatusCode::OK {
        violations.push(format!("/healthz returned {status}; expected 200"));
    }
    match serde_json::from_str::<Value>(body) {
        Ok(json) if json.get("status").and_then(Value::as_str) == Some("ok") => {}
        Ok(_) => violations.push("/healthz JSON must contain `status: \"ok\"`".to_owned()),
        Err(error) => violations.push(format!("/healthz did not return valid JSON: {error}")),
    }
}

fn check_ready(
    status: StatusCode,
    body: &str,
    expected_ready: bool,
    expected_check: Option<&str>,
    violations: &mut Vec<String>,
) {
    let expected_status = if expected_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    if status != expected_status {
        violations.push(format!(
            "/readyz returned {status}; expected {expected_status} when readiness should be {}",
            if expected_ready { "passing" } else { "failing" }
        ));
    }

    let json = match serde_json::from_str::<Value>(body) {
        Ok(json) => json,
        Err(error) => {
            violations.push(format!("/readyz did not return valid JSON: {error}"));
            return;
        }
    };
    let expected_word = if expected_ready { "ready" } else { "not_ready" };
    if json.get("status").and_then(Value::as_str) != Some(expected_word) {
        violations.push(format!(
            "/readyz JSON status must be `{expected_word}` when readiness should be {}",
            if expected_ready { "passing" } else { "failing" }
        ));
    }
    let Some(checks) = json.get("checks").and_then(Value::as_array) else {
        violations.push("/readyz JSON must contain a `checks` array".to_owned());
        return;
    };
    if let Some(expected_check) = expected_check {
        let result = checks
            .iter()
            .find(|check| check.get("name").and_then(Value::as_str) == Some(expected_check));
        match result {
            Some(result)
                if result.get("status").and_then(Value::as_str) == Some("fail")
                    && result.get("error").and_then(Value::as_str).is_some() => {}
            Some(_) => violations.push(format!(
                "/readyz check `{expected_check}` must contain `status: \"fail\"` and an error"
            )),
            None => violations.push(format!(
                "/readyz JSON has no per-check result for `{expected_check}`"
            )),
        }
    }
}

fn check_metrics(
    status: StatusCode,
    content_type: Option<&str>,
    body: &str,
    violations: &mut Vec<String>,
) {
    if status != StatusCode::OK {
        violations.push(format!("/metrics returned {status}; expected 200"));
    }
    if !content_type.is_some_and(|value| value.starts_with("text/plain")) {
        violations.push(format!(
            "/metrics content type was {content_type:?}; expected text/plain Prometheus exposition"
        ));
    }
    if !body.lines().any(|line| {
        let line = line.trim();
        !line.is_empty() && (line.starts_with('#') || line.contains(char::is_whitespace))
    }) {
        violations.push("/metrics body does not look like Prometheus text exposition".to_owned());
    }
}

fn finish(violations: Vec<String>) -> Result<(), OpsConformanceError> {
    if violations.is_empty() {
        Ok(())
    } else {
        Err(OpsConformanceError { violations })
    }
}

#[cfg(test)]
mod tests {
    use baukit_ops::OpsRouter;
    use baukit_telemetry::{DeploymentEnvironment, ProcessKind, ServiceIdentity};
    use metrics::Recorder as _;
    use metrics_exporter_prometheus::PrometheusBuilder;

    use super::*;

    #[tokio::test]
    async fn real_baukit_ops_router_conforms_in_process() {
        let recorder = PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        recorder
            .register_gauge(
                &metrics::Key::from_name("fixture_info"),
                &metrics::Metadata::new("fixture", metrics::Level::INFO, Some("fixture")),
            )
            .set(1.0);
        let readiness = ReadinessRegistry::new();
        readiness
            .register_fn_default("passing_dependency", || async { Ok(()) })
            .expect("valid readiness registration");
        let identity = ServiceIdentity::new(
            "test-product",
            ProcessKind::Api,
            "1.0.0",
            "abc123",
            DeploymentEnvironment::Local,
        );
        let router = OpsRouter::new(identity, handle)
            .with_readiness(readiness.clone())
            .into_router();

        check_ops_router_conformance(&router, &readiness)
            .await
            .expect("real Baukit operations router conforms");
    }
}
