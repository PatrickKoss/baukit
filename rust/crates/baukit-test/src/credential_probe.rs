//! Scripted HTTP responses and conformance checks for credential probes.

use std::{
    collections::{BTreeMap, VecDeque},
    fmt, io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Response, StatusCode},
};
use baukit_integrations::{
    CredentialProbe, CredentialProbeError, CredentialProbeResult,
    MAX_CREDENTIAL_PROBE_RESPONSE_BYTES,
};
use tokio::{net::TcpListener, task::JoinHandle};

const DEFAULT_RETRY_AFTER: Duration = Duration::from_secs(37);
const CONFORMANCE_CALL_DEADLINE: Duration = Duration::from_secs(2);
const PRIVATE_PROVIDER_TEXT: &str = "private-provider-response credential-probe-secret";

/// One raw HTTP response queued on [`ScriptedCredentialProbeHttp`].
///
/// The type omits `Debug` so a failed test does not print its provider body.
pub struct ScriptedCredentialProbeResponse {
    outcome: ScriptedOutcome,
}

enum ScriptedOutcome {
    Reply {
        status: u16,
        headers: BTreeMap<String, String>,
        body: Vec<u8>,
        streamed: bool,
    },
    Pending,
}

impl ScriptedCredentialProbeResponse {
    /// Creates an HTTP response with `status` and `body`.
    #[must_use]
    pub fn new(status: u16, body: impl Into<Vec<u8>>) -> Self {
        Self {
            outcome: ScriptedOutcome::Reply {
                status,
                headers: BTreeMap::new(),
                body: body.into(),
                streamed: false,
            },
        }
    }

    /// Creates a successful HTTP response.
    #[must_use]
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self::new(StatusCode::OK.as_u16(), body)
    }

    /// Adds one response header.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let ScriptedOutcome::Reply { headers, .. } = &mut self.outcome {
            headers.insert(name.into(), value.into());
        }
        self
    }

    /// Streams the body without a `Content-Length` header.
    #[must_use]
    pub fn streamed(mut self) -> Self {
        if let ScriptedOutcome::Reply { streamed, .. } = &mut self.outcome {
            *streamed = true;
        }
        self
    }

    /// Creates a response that never completes.
    ///
    /// The probe adapter must enforce its own finite timeout. The conformance
    /// runner stops waiting after two seconds and reports a violation.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            outcome: ScriptedOutcome::Pending,
        }
    }
}

#[derive(Clone)]
struct ScriptedState {
    responses: Arc<Mutex<VecDeque<ScriptedCredentialProbeResponse>>>,
    calls: Arc<AtomicUsize>,
}

/// A loopback HTTP server that returns queued responses in request order.
///
/// The server records only a call count. It discards request paths, headers,
/// bodies, and credentials so fixture diagnostics cannot expose them.
pub struct ScriptedCredentialProbeHttp {
    origin: String,
    state: ScriptedState,
    task: JoinHandle<io::Result<()>>,
}

impl ScriptedCredentialProbeHttp {
    /// Starts a scripted server on an ephemeral loopback port.
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let state = ScriptedState {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let router = Router::new()
            .fallback(scripted_response)
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            origin: format!("http://{address}"),
            state,
            task,
        })
    }

    /// Returns the loopback origin used to configure the adapter under test.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Queues one response for the next request.
    pub fn push_response(&self, response: ScriptedCredentialProbeResponse) {
        self.state
            .responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(response);
    }

    /// Returns the number of requests received without retaining request data.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }
}

impl Drop for ScriptedCredentialProbeHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn scripted_response(State(state): State<ScriptedState>) -> Response<Body> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let response = state
        .responses
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop_front();
    match response.map(|response| response.outcome) {
        Some(ScriptedOutcome::Reply {
            status,
            headers,
            body,
            streamed,
        }) => build_response(status, headers, body, streamed),
        Some(ScriptedOutcome::Pending) => std::future::pending().await,
        None => build_response(
            StatusCode::INTERNAL_SERVER_ERROR.as_u16(),
            BTreeMap::new(),
            Vec::new(),
            false,
        ),
    }
}

fn build_response(
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
    streamed: bool,
) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    let body = if streamed {
        Body::from_stream(Body::from(body).into_data_stream())
    } else {
        Body::from(body)
    };
    builder.body(body).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("static response is valid")
    })
}

/// Provider-authored raw responses used by the credential-probe check.
///
/// Start with [`new`](Self::new), then replace any response whose provider has
/// a different wire shape. The conformance runner never branches on a provider.
pub struct CredentialProbeConformanceCases {
    healthy: ScriptedCredentialProbeResponse,
    expected_external_account_id: String,
    revoked: ScriptedCredentialProbeResponse,
    missing_scope: ScriptedCredentialProbeResponse,
    rate_limited: ScriptedCredentialProbeResponse,
    rate_limited_without_hint: ScriptedCredentialProbeResponse,
    unavailable: ScriptedCredentialProbeResponse,
    invalid_data: ScriptedCredentialProbeResponse,
    oversized: ScriptedCredentialProbeResponse,
    timeout: ScriptedCredentialProbeResponse,
    expected_retry_after: Duration,
}

impl CredentialProbeConformanceCases {
    /// Creates cases with common HTTP statuses and caller-supplied success data.
    ///
    /// Replace the missing-scope response when a provider reports missing
    /// access in a successful response header or body. Replace invalid data
    /// when the adapter accepts the default private-text body as valid.
    #[must_use]
    pub fn new(
        healthy: ScriptedCredentialProbeResponse,
        expected_external_account_id: impl Into<String>,
    ) -> Self {
        Self {
            healthy,
            expected_external_account_id: expected_external_account_id.into(),
            revoked: private_response(StatusCode::UNAUTHORIZED),
            missing_scope: private_response(StatusCode::FORBIDDEN),
            rate_limited: private_response(StatusCode::TOO_MANY_REQUESTS)
                .with_header("retry-after", DEFAULT_RETRY_AFTER.as_secs().to_string()),
            rate_limited_without_hint: private_response(StatusCode::TOO_MANY_REQUESTS),
            unavailable: private_response(StatusCode::SERVICE_UNAVAILABLE),
            invalid_data: ScriptedCredentialProbeResponse::ok(PRIVATE_PROVIDER_TEXT),
            oversized: ScriptedCredentialProbeResponse::ok(vec![
                b'x';
                MAX_CREDENTIAL_PROBE_RESPONSE_BYTES
                    + 1
            ])
            .streamed(),
            timeout: ScriptedCredentialProbeResponse::pending(),
            expected_retry_after: DEFAULT_RETRY_AFTER,
        }
    }

    /// Replaces the response expected to map to a revoked credential.
    #[must_use]
    pub fn with_revoked_response(mut self, response: ScriptedCredentialProbeResponse) -> Self {
        self.revoked = response;
        self
    }

    /// Replaces the response expected to map to missing required access.
    #[must_use]
    pub fn with_missing_scope_response(
        mut self,
        response: ScriptedCredentialProbeResponse,
    ) -> Self {
        self.missing_scope = response;
        self
    }

    /// Replaces the rate-limit response and its expected retry delay.
    #[must_use]
    pub fn with_rate_limited_response(
        mut self,
        response: ScriptedCredentialProbeResponse,
        expected_retry_after: Duration,
    ) -> Self {
        self.rate_limited = response;
        self.expected_retry_after = expected_retry_after;
        self
    }

    /// Replaces the rate-limit response that contains no usable retry hint.
    #[must_use]
    pub fn with_rate_limited_without_hint_response(
        mut self,
        response: ScriptedCredentialProbeResponse,
    ) -> Self {
        self.rate_limited_without_hint = response;
        self
    }

    /// Replaces the response expected to map to provider unavailability.
    #[must_use]
    pub fn with_unavailable_response(mut self, response: ScriptedCredentialProbeResponse) -> Self {
        self.unavailable = response;
        self
    }

    /// Replaces the malformed or otherwise invalid response.
    #[must_use]
    pub fn with_invalid_data_response(mut self, response: ScriptedCredentialProbeResponse) -> Self {
        self.invalid_data = response;
        self
    }

    /// Replaces the response that exceeds the adapter's read bound.
    #[must_use]
    pub fn with_oversized_response(mut self, response: ScriptedCredentialProbeResponse) -> Self {
        self.oversized = response;
        self
    }

    /// Replaces the response used to prove the adapter's finite timeout.
    #[must_use]
    pub fn with_timeout_response(mut self, response: ScriptedCredentialProbeResponse) -> Self {
        self.timeout = response;
        self
    }
}

fn private_response(status: StatusCode) -> ScriptedCredentialProbeResponse {
    ScriptedCredentialProbeResponse::new(status.as_u16(), PRIVATE_PROVIDER_TEXT)
}

/// Violations found while exercising a credential-probe adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialProbeConformanceError {
    violations: Vec<String>,
}

impl CredentialProbeConformanceError {
    /// Returns violations in probe-case order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for CredentialProbeConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "credential-probe conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for CredentialProbeConformanceError {}

/// Checks one product adapter against the credential-probe outcome contract.
///
/// `make_probe` receives the scripted server origin. Configure a fresh adapter
/// to call any path below that origin with a timeout shorter than two seconds.
/// The supplied cases retain all provider headers and response parsing in the
/// product test.
pub async fn check_credential_probe_conformance<MakeProbe, Probe>(
    cases: CredentialProbeConformanceCases,
    make_probe: MakeProbe,
) -> Result<(), CredentialProbeConformanceError>
where
    MakeProbe: FnOnce(&str) -> Probe,
    Probe: CredentialProbe,
{
    let server = ScriptedCredentialProbeHttp::start()
        .await
        .map_err(|_| fixture_failure("could not start scripted HTTP server"))?;
    let probe = make_probe(server.origin());
    let mut violations = Vec::new();

    server.push_response(cases.healthy);
    match invoke(&probe).await {
        Some(Ok(success)) => {
            if success.external_account_id.as_str() != cases.expected_external_account_id {
                violations.push("healthy response returned a different external account ID".into());
            }
            if success.health() != baukit_integrations::ConnectionHealth::Healthy {
                violations.push("healthy response did not return healthy connection state".into());
            }
        }
        Some(Err(error)) => violations.push(format!(
            "healthy response returned failure class {}",
            error.code()
        )),
        None => violations.push("healthy response exceeded the conformance deadline".into()),
    }

    check_failure(
        &server,
        &probe,
        "revoked",
        cases.revoked,
        CredentialProbeError::Revoked,
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "missing scope",
        cases.missing_scope,
        CredentialProbeError::MissingScope,
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "rate limited with Retry-After",
        cases.rate_limited,
        CredentialProbeError::rate_limited(Some(cases.expected_retry_after)),
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "rate limited without Retry-After",
        cases.rate_limited_without_hint,
        CredentialProbeError::rate_limited(None),
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "unavailable",
        cases.unavailable,
        CredentialProbeError::Unavailable,
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "invalid data",
        cases.invalid_data,
        CredentialProbeError::InvalidData,
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "oversized response",
        cases.oversized,
        CredentialProbeError::InvalidData,
        &mut violations,
    )
    .await;
    check_failure(
        &server,
        &probe,
        "timeout",
        cases.timeout,
        CredentialProbeError::Timeout,
        &mut violations,
    )
    .await;

    if server.calls() != 9 {
        violations.push(format!(
            "scripted HTTP server received {} calls; expected 9",
            server.calls()
        ));
    }
    finish(violations)
}

/// Panics when a credential-probe adapter violates the contract.
///
/// # Panics
///
/// Panics with every conformance violation or fixture failure.
pub async fn assert_credential_probe_conformance<MakeProbe, Probe>(
    cases: CredentialProbeConformanceCases,
    make_probe: MakeProbe,
) where
    MakeProbe: FnOnce(&str) -> Probe,
    Probe: CredentialProbe,
{
    if let Err(error) = check_credential_probe_conformance(cases, make_probe).await {
        panic!("{error}");
    }
}

async fn check_failure<Probe: CredentialProbe>(
    server: &ScriptedCredentialProbeHttp,
    probe: &Probe,
    case: &str,
    response: ScriptedCredentialProbeResponse,
    expected: CredentialProbeError,
    violations: &mut Vec<String>,
) {
    server.push_response(response);
    match invoke(probe).await {
        Some(Err(actual)) if actual == expected => {
            let display = actual.to_string();
            let debug = format!("{actual:?}");
            if display.contains(PRIVATE_PROVIDER_TEXT) || debug.contains(PRIVATE_PROVIDER_TEXT) {
                violations.push(format!("{case} exposed provider response text"));
            }
        }
        Some(Err(actual)) => violations.push(format!(
            "{case} returned failure class {}; expected {}",
            actual.code(),
            expected.code()
        )),
        Some(Ok(_)) => violations.push(format!("{case} response was accepted")),
        None => violations.push(format!("{case} response exceeded the conformance deadline")),
    }
}

async fn invoke(probe: &impl CredentialProbe) -> Option<CredentialProbeResult> {
    tokio::time::timeout(
        CONFORMANCE_CALL_DEADLINE,
        probe.probe(b"credential-probe-conformance-secret"),
    )
    .await
    .ok()
}

fn fixture_failure(message: &str) -> CredentialProbeConformanceError {
    CredentialProbeConformanceError {
        violations: vec![message.to_owned()],
    }
}

fn finish(violations: Vec<String>) -> Result<(), CredentialProbeConformanceError> {
    if violations.is_empty() {
        Ok(())
    } else {
        Err(CredentialProbeConformanceError { violations })
    }
}

#[cfg(test)]
mod tests {
    use baukit_http::retry_after_from_headers;
    use serde_json::Value;

    use super::*;
    use baukit_integrations::{CredentialProbeFuture, CredentialProbeSuccess, ExternalAccountId};

    struct HttpProbe {
        client: reqwest::Client,
        endpoint: String,
        required_scope: Option<&'static str>,
    }

    impl HttpProbe {
        fn new(origin: &str, required_scope: Option<&'static str>) -> Self {
            Self {
                client: reqwest::Client::builder()
                    .timeout(Duration::from_millis(500))
                    .build()
                    .expect("test client builds"),
                endpoint: format!("{origin}/credential-check"),
                required_scope,
            }
        }

        async fn execute(&self, credential: &[u8]) -> CredentialProbeResult {
            let credential =
                std::str::from_utf8(credential).map_err(|_| CredentialProbeError::InvalidData)?;
            let response = self
                .client
                .get(&self.endpoint)
                .header("x-test-credential", credential)
                .send()
                .await
                .map_err(|error| {
                    if error.is_timeout() {
                        CredentialProbeError::Timeout
                    } else {
                        CredentialProbeError::Unavailable
                    }
                })?;
            match response.status() {
                StatusCode::UNAUTHORIZED => return Err(CredentialProbeError::Revoked),
                StatusCode::FORBIDDEN => return Err(CredentialProbeError::MissingScope),
                StatusCode::TOO_MANY_REQUESTS => {
                    return Err(CredentialProbeError::rate_limited(
                        retry_after_from_headers(response.headers(), &[]),
                    ));
                }
                status if status.is_server_error() => {
                    return Err(CredentialProbeError::Unavailable);
                }
                status if !status.is_success() => {
                    return Err(CredentialProbeError::InvalidData);
                }
                _ => {}
            }
            if let Some(scope) = self.required_scope {
                let has_scope = response
                    .headers()
                    .get("x-test-scopes")
                    .and_then(|value| value.to_str().ok())
                    .is_some_and(|scopes| {
                        scopes.split(',').map(str::trim).any(|item| item == scope)
                    });
                if !has_scope {
                    return Err(CredentialProbeError::MissingScope);
                }
            }
            let body = read_bounded(response).await?;
            let body: Value =
                serde_json::from_slice(&body).map_err(|_| CredentialProbeError::InvalidData)?;
            let id = body
                .get("id")
                .and_then(|value| match value {
                    Value::Number(value) => Some(value.to_string()),
                    Value::String(value) => Some(value.clone()),
                    _ => None,
                })
                .ok_or(CredentialProbeError::InvalidData)?;
            let account_id =
                ExternalAccountId::new(id).map_err(|_| CredentialProbeError::InvalidData)?;
            Ok(CredentialProbeSuccess::new(account_id))
        }
    }

    impl CredentialProbe for HttpProbe {
        fn probe<'a>(&'a self, credential: &'a [u8]) -> CredentialProbeFuture<'a> {
            Box::pin(async move { self.execute(credential).await })
        }
    }

    async fn read_bounded(mut response: reqwest::Response) -> CredentialProbeResultBody {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_CREDENTIAL_PROBE_RESPONSE_BYTES as u64)
        {
            return Err(CredentialProbeError::InvalidData);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| CredentialProbeError::InvalidData)?
        {
            if body.len().saturating_add(chunk.len()) > MAX_CREDENTIAL_PROBE_RESPONSE_BYTES {
                return Err(CredentialProbeError::InvalidData);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    type CredentialProbeResultBody = Result<Vec<u8>, CredentialProbeError>;

    #[tokio::test]
    async fn header_scoped_adapter_passes_the_shared_suite() {
        let healthy = ScriptedCredentialProbeResponse::ok(br#"{"id":321}"#)
            .with_header("x-test-scopes", "records:read, profile");
        let cases = CredentialProbeConformanceCases::new(healthy, "321")
            .with_missing_scope_response(ScriptedCredentialProbeResponse::ok(br#"{"id":321}"#))
            .with_invalid_data_response(
                ScriptedCredentialProbeResponse::ok(PRIVATE_PROVIDER_TEXT)
                    .with_header("x-test-scopes", "records:read"),
            )
            .with_oversized_response(
                ScriptedCredentialProbeResponse::ok(vec![
                    b'x';
                    MAX_CREDENTIAL_PROBE_RESPONSE_BYTES + 1
                ])
                .with_header("x-test-scopes", "records:read")
                .streamed(),
            );

        check_credential_probe_conformance(cases, |origin| {
            HttpProbe::new(origin, Some("records:read"))
        })
        .await
        .expect("header-scoped adapter conforms");
    }

    #[tokio::test]
    async fn status_scoped_adapter_passes_the_shared_suite() {
        let healthy = ScriptedCredentialProbeResponse::ok(br#"{"id":"account-b"}"#);
        let cases = CredentialProbeConformanceCases::new(healthy, "account-b");

        assert_credential_probe_conformance(cases, |origin| HttpProbe::new(origin, None)).await;
    }

    #[tokio::test]
    async fn fake_http_discards_request_details() {
        let server = ScriptedCredentialProbeHttp::start()
            .await
            .expect("server starts");
        server.push_response(ScriptedCredentialProbeResponse::ok("{}"));
        reqwest::Client::new()
            .get(format!("{}/private-path", server.origin()))
            .header("authorization", "Bearer private-token")
            .send()
            .await
            .expect("request completes");

        assert_eq!(server.calls(), 1);
    }

    #[tokio::test]
    async fn fake_http_can_stream_without_a_content_length() {
        let server = ScriptedCredentialProbeHttp::start()
            .await
            .expect("server starts");
        server.push_response(ScriptedCredentialProbeResponse::ok("streamed").streamed());
        let response = reqwest::get(server.origin())
            .await
            .expect("request completes");

        assert_eq!(response.content_length(), None);
        assert_eq!(response.text().await.expect("body reads"), "streamed");
    }
}
