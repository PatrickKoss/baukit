//! Webhook signing and scripted receiver fixtures.

use std::{
    collections::{BTreeMap, VecDeque},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Request, State},
    http::{HeaderMap, Method, Response, StatusCode, Uri},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::hmac;
use tokio::{net::TcpListener, task::JoinHandle};

const SIGNING_VERSION: &[u8] = b"baukit-webhook-v1\n";
const SIGNATURE_PREFIX: &str = "v1=";

/// Maximum request body retained by [`ScriptedWebhookReceiver`].
pub const MAX_SCRIPTED_WEBHOOK_BODY_BYTES: usize = 1_048_576;

/// Builds the exact bytes covered by the webhook HMAC helper.
///
/// The input is the fixed version line, decimal Unix timestamp, decimal byte
/// length of the delivery ID, delivery ID, and raw request body. Each field
/// before the raw body ends with `\n`. The length makes the two variable fields
/// unambiguous.
#[must_use]
pub fn webhook_signing_input(timestamp: i64, delivery_id: &str, body: &[u8]) -> Vec<u8> {
    let mut input = Vec::new();
    input.extend_from_slice(SIGNING_VERSION);
    input.extend_from_slice(timestamp.to_string().as_bytes());
    input.push(b'\n');
    input.extend_from_slice(delivery_id.len().to_string().as_bytes());
    input.push(b'\n');
    input.extend_from_slice(delivery_id.as_bytes());
    input.push(b'\n');
    input.extend_from_slice(body);
    input
}

/// Signs one webhook request with HMAC-SHA256.
///
/// The returned header value starts with `v1=` and uses unpadded base64url.
/// The helper does not retain the secret.
#[must_use]
pub fn sign_webhook_hmac_sha256(
    secret: &[u8],
    timestamp: i64,
    delivery_id: &str,
    body: &[u8],
) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, &webhook_signing_input(timestamp, delivery_id, body));
    format!("{SIGNATURE_PREFIX}{}", URL_SAFE_NO_PAD.encode(tag.as_ref()))
}

/// Verifies a webhook signature against current and retained rotation keys.
///
/// Invalid encodings return `false`. Callers should use a non-secret key ID to
/// select a bounded set of candidates, normally the current and previous key.
#[must_use]
pub fn verify_webhook_hmac_sha256<'a>(
    candidate_secrets: impl IntoIterator<Item = &'a [u8]>,
    timestamp: i64,
    delivery_id: &str,
    body: &[u8],
    signature: &str,
) -> bool {
    let Some(encoded) = signature.strip_prefix(SIGNATURE_PREFIX) else {
        return false;
    };
    let Ok(supplied) = URL_SAFE_NO_PAD.decode(encoded) else {
        return false;
    };
    let input = webhook_signing_input(timestamp, delivery_id, body);
    candidate_secrets.into_iter().any(|secret| {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        hmac::verify(&key, &input, &supplied).is_ok()
    })
}

/// One response queued on a [`ScriptedWebhookReceiver`].
#[derive(Clone)]
pub struct ScriptedWebhookResponse {
    outcome: ScriptedOutcome,
}

#[derive(Clone)]
enum ScriptedOutcome {
    Reply {
        status: u16,
        headers: BTreeMap<String, String>,
    },
    Pending,
}

impl ScriptedWebhookResponse {
    /// Creates an empty response with the supplied status.
    #[must_use]
    pub fn new(status: u16) -> Self {
        Self {
            outcome: ScriptedOutcome::Reply {
                status,
                headers: BTreeMap::new(),
            },
        }
    }

    /// Creates a successful empty response.
    #[must_use]
    pub fn success() -> Self {
        Self::new(StatusCode::NO_CONTENT.as_u16())
    }

    /// Adds one response header, such as `Retry-After`.
    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        if let ScriptedOutcome::Reply { headers, .. } = &mut self.outcome {
            headers.insert(name.into(), value.into());
        }
        self
    }

    /// Creates a response that never completes.
    ///
    /// Delivery code must enforce its own finite timeout.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            outcome: ScriptedOutcome::Pending,
        }
    }
}

/// One bounded request captured by [`ScriptedWebhookReceiver`].
///
/// This type omits `Debug` so assertion output cannot print a signature or
/// private event body by accident.
#[derive(Clone)]
pub struct ReceivedWebhookRequest {
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl ReceivedWebhookRequest {
    /// Returns the request method.
    #[must_use]
    pub const fn method(&self) -> &Method {
        &self.method
    }

    /// Returns the request URI, including its path and query.
    #[must_use]
    pub const fn uri(&self) -> &Uri {
        &self.uri
    }

    /// Returns a header as UTF-8 without formatting the other headers.
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name)?.to_str().ok()
    }

    /// Returns the raw body bytes used for signature verification.
    #[must_use]
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

#[derive(Clone)]
struct ReceiverState {
    responses: Arc<Mutex<VecDeque<ScriptedWebhookResponse>>>,
    requests: Arc<Mutex<Vec<ReceivedWebhookRequest>>>,
    calls: Arc<AtomicUsize>,
}

/// Loopback HTTP receiver that plays queued delivery responses in order.
///
/// It captures bounded requests so a test can check stable bodies, delivery
/// IDs, timestamps, signatures, and retry behavior. The type omits `Debug`
/// because those values may contain private event data.
pub struct ScriptedWebhookReceiver {
    origin: String,
    state: ReceiverState,
    task: JoinHandle<io::Result<()>>,
}

impl ScriptedWebhookReceiver {
    /// Starts a receiver on an ephemeral loopback port.
    pub async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let state = ReceiverState {
            responses: Arc::new(Mutex::new(VecDeque::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            calls: Arc::new(AtomicUsize::new(0)),
        };
        let router = Router::new().fallback(receive).with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            origin: format!("http://{address}"),
            state,
            task,
        })
    }

    /// Returns the loopback origin.
    #[must_use]
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// Returns a URL below the receiver origin.
    #[must_use]
    pub fn url(&self, path: &str) -> String {
        format!("{}{}", self.origin, normalized_path(path))
    }

    /// Queues one response for the next request.
    pub fn push_response(&self, response: ScriptedWebhookResponse) {
        self.state
            .responses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(response);
    }

    /// Returns the total request count, including bodies rejected as too large.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.state.calls.load(Ordering::SeqCst)
    }

    /// Returns captured requests without logging them.
    #[must_use]
    pub fn received_requests(&self) -> Vec<ReceivedWebhookRequest> {
        self.state
            .requests
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl Drop for ScriptedWebhookReceiver {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn normalized_path(path: &str) -> String {
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

async fn receive(State(state): State<ReceiverState>, request: Request) -> Response<Body> {
    state.calls.fetch_add(1, Ordering::SeqCst);
    let (parts, body) = request.into_parts();
    let Ok(body) = to_bytes(body, MAX_SCRIPTED_WEBHOOK_BODY_BYTES).await else {
        return empty_response(StatusCode::PAYLOAD_TOO_LARGE.as_u16(), BTreeMap::new());
    };
    state
        .requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(ReceivedWebhookRequest {
            method: parts.method,
            uri: parts.uri,
            headers: parts.headers,
            body: body.to_vec(),
        });
    let response = state
        .responses
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .pop_front();
    match response.map(|response| response.outcome) {
        Some(ScriptedOutcome::Reply { status, headers }) => empty_response(status, headers),
        Some(ScriptedOutcome::Pending) => std::future::pending().await,
        None => empty_response(StatusCode::NO_CONTENT.as_u16(), BTreeMap::new()),
    }
}

fn empty_response(status: u16, headers: BTreeMap<String, String>) -> Response<Body> {
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        builder = builder.header(name, value);
    }
    builder.body(Body::empty()).unwrap_or_else(|_| {
        Response::builder()
            .status(StatusCode::INTERNAL_SERVER_ERROR)
            .body(Body::empty())
            .expect("static response is valid")
    })
}

#[cfg(test)]
mod tests {
    use reqwest::Client;

    use super::*;

    #[test]
    fn signing_input_is_stable_and_rotation_accepts_the_previous_key() {
        let body = br#"{"event":"created"}"#;
        let signature =
            sign_webhook_hmac_sha256(b"current-secret", 1_800_000_000, "delivery-7", body);

        assert_eq!(signature, "v1=UpNJdPkf1wS7p7DY75L8nz7Rz_BUPFFlEOX3ma4py7w");
        assert!(verify_webhook_hmac_sha256(
            [b"previous-secret".as_slice(), b"current-secret".as_slice()],
            1_800_000_000,
            "delivery-7",
            body,
            &signature,
        ));
        assert!(!verify_webhook_hmac_sha256(
            [b"previous-secret".as_slice()],
            1_800_000_000,
            "delivery-7",
            body,
            &signature,
        ));
        assert!(!verify_webhook_hmac_sha256(
            [b"current-secret".as_slice()],
            1_800_000_001,
            "delivery-7",
            body,
            &signature,
        ));
    }

    #[tokio::test]
    async fn receiver_scripts_retry_and_captures_exact_requests()
    -> Result<(), Box<dyn std::error::Error>> {
        let receiver = ScriptedWebhookReceiver::start().await?;
        receiver.push_response(
            ScriptedWebhookResponse::new(StatusCode::TOO_MANY_REQUESTS.as_u16())
                .with_header("retry-after", "17"),
        );
        receiver.push_response(ScriptedWebhookResponse::success());
        let client = Client::new();
        let body = br#"{"private":"fixture"}"#;
        let first = client
            .post(receiver.url("events"))
            .header("idempotency-key", "delivery-1")
            .body(body.as_slice())
            .send()
            .await?;
        let second = client
            .post(receiver.url("events"))
            .header("idempotency-key", "delivery-1")
            .body(body.as_slice())
            .send()
            .await?;

        assert_eq!(first.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            first
                .headers()
                .get("retry-after")
                .and_then(|value| value.to_str().ok()),
            Some("17")
        );
        assert_eq!(second.status(), StatusCode::NO_CONTENT);
        assert_eq!(receiver.calls(), 2);
        let requests = receiver.received_requests();
        assert_eq!(requests.len(), 2);
        assert_eq!(requests[0].method(), Method::POST);
        assert_eq!(requests[0].uri().path(), "/events");
        assert_eq!(requests[0].header("idempotency-key"), Some("delivery-1"));
        assert_eq!(requests[0].body(), requests[1].body());
        Ok(())
    }
}
