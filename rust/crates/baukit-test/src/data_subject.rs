use std::{
    collections::HashMap,
    io,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use aws_lc_rs::{
    digest::{SHA256, digest},
    hmac,
    rand::SystemRandom,
    signature::Ed25519KeyPair,
};
use axum::{
    Json, Router,
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, Method, StatusCode, header},
    response::{IntoResponse, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::{
    CONTRACT_VERSION, CategoryEntry, DataSubjectRequest, DiscoveryDocument, ExportPart,
    IdentifierType, Limits, OperationError, OperationKind, OperationResource, OperationStatus,
    RECEIPT_TYPE, ReceiptKey, ReceiptKeyStatus, ReceiptPayload, ReceiptSigner, Restriction, Scopes,
    Service, codes, normalize_request, validate_idempotency_key, validate_subject_reference,
};
use baukit_http::ApiError;
use chrono::{DateTime, Utc};
use serde_json::Value;
use tokio::{net::TcpListener, sync::watch, task::JoinHandle};
use uuid::Uuid;

const DISCOVERY: &str = "/.well-known/baukit/data-subject";
const PREFIX: &str = "/api/v1/data-subject/";
const MAX_REQUEST_BYTES: usize = 1_048_576;

/// Outcome applied when an operation finishes. Export content stays in memory only.
/// This type omits `Debug` to keep exported data out of assertion output.
#[derive(Clone)]
pub enum DataSubjectOutcome {
    /// Complete with the supplied category counts.
    Completed {
        /// Safe category names, actions and counts.
        categories: Vec<CategoryEntry>,
    },
    /// Return a signed no-match result.
    NotFound,
    /// Complete erasure with legal retention obligations.
    Restricted {
        /// Erased and retained category counts.
        categories: Vec<CategoryEntry>,
        /// Safe explanations of retained categories.
        restrictions: Vec<Restriction>,
    },
    /// Return pending for this many status GETs, then apply the nested outcome.
    /// POSTs, replays and part downloads do not consume polls. Nesting is supported.
    PendingFor {
        /// Number of pending status GETs.
        polls: usize,
        /// Outcome after the pending polls.
        then: Box<Self>,
    },
    /// Reach a terminal failure without a receipt.
    Failed {
        /// Safe stable service error code, without identifiers or internal causes.
        code: String,
    },
    /// Complete an export with compact JSON parts, numbered starting at one.
    Export {
        /// Each value becomes one bounded downloadable JSON part.
        parts: Vec<Value>,
    },
}

/// Deliberate protocol faults. Receipt faults apply once at completion and survive replay.
#[derive(Clone, Debug)]
pub enum DataSubjectFault {
    /// Sign a valid envelope with a key absent from discovery.
    BadSignature,
    /// Replace the envelope's key ID with an unadvertised ID.
    WrongKeyId,
    /// Change valid JSON payload bytes after signing.
    TamperedPayload,
    /// Sign a valid receipt containing a different syntactically valid reference.
    WrongSubjectReference,
    /// Serve different JSON bytes with the same length as the signed part metadata.
    PartDigestMismatch,
    /// Delay every response using this script, after processing the request.
    Delay(Duration),
    /// Return this 5xx status for the next `times` authorized, valid requests.
    ServerError {
        /// An HTTP status in 500..=599.
        status: u16,
        /// Remaining responses before normal processing resumes.
        times: usize,
    },
    /// Return 429 with a delta-seconds Retry-After header for `times` requests.
    RateLimited {
        /// Retry interval, rounded up to whole seconds.
        retry_after: Duration,
        /// Remaining responses before normal processing resumes.
        times: usize,
    },
}

/// Outcome and faults shared by requests selecting one subject script.
/// Transient fault counters are shared across its operations and both operation kinds.
#[derive(Clone)]
pub struct DataSubjectScript {
    outcome: DataSubjectOutcome,
    faults: Vec<DataSubjectFault>,
}

impl From<DataSubjectOutcome> for DataSubjectScript {
    fn from(outcome: DataSubjectOutcome) -> Self {
        Self {
            outcome,
            faults: Vec::new(),
        }
    }
}

impl DataSubjectScript {
    /// Creates a script with no faults.
    pub fn new(outcome: DataSubjectOutcome) -> Self {
        outcome.into()
    }

    /// Adds a fault. Transient responses are consumed in insertion order.
    #[must_use]
    pub fn with_fault(mut self, fault: DataSubjectFault) -> Self {
        self.faults.push(fault);
        self
    }
}

/// Safe script configuration failure, without request or export content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataSubjectScriptError {
    /// Subject reference does not satisfy the contract's syntax.
    #[error("invalid test subject reference")]
    SubjectReference,
    /// A part exceeds the advertised size or part numbers cannot fit u32.
    #[error("invalid test export part bounds")]
    ExportBounds,
    /// A ServerError status is outside 500..=599.
    #[error("invalid test server error status")]
    ServerStatus,
    /// Fault injection requires an explicitly configured subject script.
    #[error("test subject script is not configured")]
    MissingScript,
}

/// Discovery and retention settings for a controlled target.
#[derive(Clone, Debug)]
pub struct DataSubjectTargetOptions {
    /// Safe service identity included in discovery and receipts.
    pub service: Service,
    /// Accepted identifier types.
    pub identifier_types: Vec<IdentifierType>,
    /// Advertised request and export bounds.
    pub limits: Limits,
    /// Part lifetime after completion, seven days by default.
    pub export_retention: Duration,
    /// Operation and replay lifetime after creation, 400 days by default.
    pub operation_retention: Duration,
}

impl Default for DataSubjectTargetOptions {
    fn default() -> Self {
        Self {
            service: Service {
                name: "data-subject-test-target".into(),
                version: "1".into(),
            },
            identifier_types: vec![
                IdentifierType::Email,
                IdentifierType::AccountId,
                IdentifierType::CustomerId,
                IdentifierType::Phone,
            ],
            limits: Limits::default(),
            export_retention: Duration::from_secs(7 * 24 * 60 * 60),
            operation_retention: Duration::from_secs(400 * 24 * 60 * 60),
        }
    }
}

/// In-memory token registration handle. It contains no bearer credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DataSubjectTokenId(u64);

/// Captured HTTP request, with no `Debug` or serialization implementation.
/// Bodies and idempotency keys are test observations. Never write them to logs.
#[derive(Clone)]
pub struct ReceivedDataSubjectRequest {
    method: Method,
    path: String,
    idempotency_key: Option<String>,
    token: Option<DataSubjectTokenId>,
    body: Option<Value>,
}

impl ReceivedDataSubjectRequest {
    /// HTTP method.
    pub fn method(&self) -> &Method {
        &self.method
    }
    /// Path without query parameters.
    pub fn path(&self) -> &str {
        &self.path
    }
    /// First UTF-8 idempotency header, if supplied.
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }
    /// Registration used, or None for missing, malformed or unknown authorization.
    pub fn token(&self) -> Option<DataSubjectTokenId> {
        self.token
    }
    /// Parsed JSON body, including bodies that fail typed contract validation.
    pub fn body(&self) -> Option<&Value> {
        self.body.as_ref()
    }
}

struct Token {
    id: DataSubjectTokenId,
    client: String,
    scopes: Vec<String>,
}

type SharedScript = Arc<Mutex<DataSubjectScript>>;

struct Operation {
    client: String,
    key: String,
    request_hmac: Vec<u8>,
    resource: OperationResource,
    outcome: DataSubjectOutcome,
    script: SharedScript,
    parts: Vec<Vec<u8>>,
}

struct TargetState {
    options: DataSubjectTargetOptions,
    discovery: DiscoveryDocument,
    signer: ReceiptSigner,
    unadvertised_signer: ReceiptSigner,
    hmac_key: hmac::Key,
    tokens: HashMap<String, Token>,
    next_token: u64,
    scripts: HashMap<String, SharedScript>,
    default_script: SharedScript,
    operations: HashMap<Uuid, Operation>,
    requests: Vec<ReceivedDataSubjectRequest>,
    clock_offset: chrono::Duration,
}

impl TargetState {
    fn now(&self) -> DateTime<Utc> {
        Utc::now() + self.clock_offset
    }

    fn purge(&mut self) {
        let now = self.now();
        self.operations.retain(|_, operation| {
            if operation.resource.completed_at.is_some_and(|completed| {
                now.signed_duration_since(completed)
                    .to_std()
                    .is_ok_and(|age| age >= self.options.export_retention)
            }) {
                operation.parts.clear();
            }
            now.signed_duration_since(operation.resource.created_at)
                .to_std()
                .is_ok_and(|age| age < self.options.operation_retention)
        });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Loopback data subject service for client tests, using real DSSE signing.
/// Identifiers, token credentials and export content remain in memory. Drop
/// closes the listener and cancels delayed responses through graceful shutdown.
pub struct DataSubjectTargetServer {
    base_url: String,
    state: Arc<Mutex<TargetState>>,
    shutdown: watch::Sender<bool>,
    _task: JoinHandle<io::Result<()>>,
}

impl DataSubjectTargetServer {
    /// Binds 127.0.0.1:0. Unscripted subjects complete with empty categories.
    pub async fn start() -> io::Result<Self> {
        Self::start_with_options(DataSubjectTargetOptions::default()).await
    }

    /// Starts with custom discovery bounds and retention settings.
    pub async fn start_with_options(options: DataSubjectTargetOptions) -> io::Result<Self> {
        let signer = fresh_signer()?;
        let unadvertised_signer = fresh_signer()?;
        let hmac_key = hmac::Key::generate(hmac::HMAC_SHA256, &SystemRandom::new())
            .map_err(|_| io::Error::other("test request hash key generation failed"))?;
        let discovery = DiscoveryDocument {
            contract_version: CONTRACT_VERSION.into(),
            service: options.service.clone(),
            operations: vec![OperationKind::Erasure, OperationKind::Export],
            identifier_types: options.identifier_types.clone(),
            scopes: Scopes::default(),
            limits: options.limits,
            receipt_keys: vec![signer.receipt_key().clone()],
        };
        let state = Arc::new(Mutex::new(TargetState {
            options,
            discovery,
            signer,
            unadvertised_signer,
            hmac_key,
            tokens: HashMap::new(),
            next_token: 0,
            scripts: HashMap::new(),
            default_script: Arc::new(Mutex::new(
                DataSubjectOutcome::Completed {
                    categories: Vec::new(),
                }
                .into(),
            )),
            operations: HashMap::new(),
            requests: Vec::new(),
            clock_offset: chrono::Duration::zero(),
        }));
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let base_url = format!("http://{}", listener.local_addr()?);
        let (shutdown, mut receiver) = watch::channel(false);
        let router = Router::new()
            .fallback(receive)
            .with_state((state.clone(), receiver.clone()));
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    let _ = receiver.changed().await;
                })
                .await
        });
        Ok(Self {
            base_url,
            state,
            shutdown,
            _task: task,
        })
    }

    /// Origin without a trailing slash.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Current discovery, including all retained signing keys.
    pub fn discovery(&self) -> DiscoveryDocument {
        lock(&self.state).discovery.clone()
    }

    /// Current public signing key.
    pub fn receipt_key(&self) -> ReceiptKey {
        lock(&self.state).signer.receipt_key().clone()
    }

    /// Registers an allowed machine token. Re-registering replaces the credential's identity.
    /// Tokens of the same client share visibility and idempotency records.
    pub fn register_token(
        &self,
        bearer: impl Into<String>,
        client_id: impl Into<String>,
        scopes: impl IntoIterator<Item = impl Into<String>>,
    ) -> DataSubjectTokenId {
        let mut state = lock(&self.state);
        let id = DataSubjectTokenId(state.next_token);
        state.next_token += 1;
        state.tokens.insert(
            bearer.into(),
            Token {
                id,
                client: client_id.into(),
                scopes: scopes.into_iter().map(Into::into).collect(),
            },
        );
        id
    }

    /// Replaces the script for future operations with this reference.
    /// Existing operations keep their original script and transient counters.
    pub fn set_script(
        &self,
        subject_reference: impl Into<String>,
        script: impl Into<DataSubjectScript>,
    ) -> Result<(), DataSubjectScriptError> {
        let reference = subject_reference.into();
        validate_subject_reference(&reference)
            .map_err(|_| DataSubjectScriptError::SubjectReference)?;
        let script = script.into();
        let mut state = lock(&self.state);
        validate_script(&script, &state.options.limits)?;
        state
            .scripts
            .insert(reference, Arc::new(Mutex::new(script)));
        Ok(())
    }

    /// Replaces the fallback for future operations without a subject-specific script.
    pub fn set_default_script(
        &self,
        script: impl Into<DataSubjectScript>,
    ) -> Result<(), DataSubjectScriptError> {
        let script = script.into();
        let mut state = lock(&self.state);
        validate_script(&script, &state.options.limits)?;
        state.default_script = Arc::new(Mutex::new(script));
        Ok(())
    }

    /// Appends a fault to an explicit subject script and operations using that script.
    /// Use after POST to exercise polling or download retries.
    pub fn push_fault(
        &self,
        subject_reference: &str,
        fault: DataSubjectFault,
    ) -> Result<(), DataSubjectScriptError> {
        let state = lock(&self.state);
        let script = state
            .scripts
            .get(subject_reference)
            .ok_or(DataSubjectScriptError::MissingScript)?;
        let mut script = lock(script);
        let mut updated = script.clone();
        updated.faults.push(fault);
        validate_script(&updated, &state.options.limits)?;
        *script = updated;
        Ok(())
    }

    /// Publishes a fresh current key and retains every old key as previous.
    /// Receipts signed before rotation remain unchanged, including on replay.
    pub fn rotate_key(&self) -> io::Result<ReceiptKey> {
        let signer = fresh_signer()?;
        let key = signer.receipt_key().clone();
        let mut state = lock(&self.state);
        for previous in &mut state.discovery.receipt_keys {
            previous.status = ReceiptKeyStatus::Previous;
        }
        state.discovery.receipt_keys.insert(0, key.clone());
        state.signer = signer;
        Ok(key)
    }

    /// Advances the retention clock without sleeping. Network delays use real time.
    /// Durations beyond chrono's range are rejected without changing the clock.
    pub fn advance_time(&self, duration: Duration) -> io::Result<()> {
        let delta = chrono::Duration::from_std(duration)
            .map_err(|_| io::Error::other("invalid test clock advance"))?;
        let mut state = lock(&self.state);
        let offset = state
            .clock_offset
            .checked_add(&delta)
            .ok_or_else(|| io::Error::other("invalid test clock advance"))?;
        Utc::now()
            .checked_add_signed(offset)
            .ok_or_else(|| io::Error::other("invalid test clock advance"))?;
        state.clock_offset = offset;
        state.purge();
        Ok(())
    }

    /// Returns observations without logging or serializing private request data.
    pub fn received_requests(&self) -> Vec<ReceivedDataSubjectRequest> {
        lock(&self.state).requests.clone()
    }
}

impl Drop for DataSubjectTargetServer {
    fn drop(&mut self) {
        self.shutdown.send_replace(true);
    }
}

fn fresh_signer() -> io::Result<ReceiptSigner> {
    let pair = Ed25519KeyPair::generate()
        .map_err(|_| io::Error::other("test signing key generation failed"))?;
    let der = pair
        .to_pkcs8()
        .map_err(|_| io::Error::other("test signing key encoding failed"))?;
    ReceiptSigner::from_pkcs8_der(der.as_ref())
        .map_err(|_| io::Error::other("test signing key loading failed"))
}

fn validate_script(
    script: &DataSubjectScript,
    limits: &Limits,
) -> Result<(), DataSubjectScriptError> {
    let mut outcome = &script.outcome;
    while let DataSubjectOutcome::PendingFor { then, .. } = outcome {
        outcome = then;
    }
    if let DataSubjectOutcome::Export { parts } = outcome
        && (u32::try_from(parts.len()).is_err()
            || parts.iter().any(|part| {
                serde_json::to_vec(part).map_or(true, |bytes| {
                    bytes.len() as u64 > limits.max_export_part_bytes
                })
            }))
    {
        return Err(DataSubjectScriptError::ExportBounds);
    }
    if script.faults.iter().any(|fault| matches!(fault, DataSubjectFault::ServerError { status, .. } if !(500..=599).contains(status))) {
        return Err(DataSubjectScriptError::ServerStatus);
    }
    Ok(())
}

async fn receive(
    State((state, mut shutdown)): State<(Arc<Mutex<TargetState>>, watch::Receiver<bool>)>,
    request: Request,
) -> Response {
    if *shutdown.borrow() {
        return error(StatusCode::SERVICE_UNAVAILABLE, "unavailable");
    }
    tokio::select! {
        biased;
        _ = shutdown.changed() => error(StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
        response = respond(state, request) => response,
    }
}

async fn respond(state: Arc<Mutex<TargetState>>, request: Request) -> Response {
    let (request, body) = request.into_parts();
    let body = to_bytes(body, MAX_REQUEST_BYTES).await;
    let parsed = body
        .as_ref()
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(bytes).ok());
    let (response, delay) = {
        let mut state = lock(&state);
        let token = registered_token(&request.headers, &state.tokens);
        let token_id = token.map(|token| token.id);
        let identity = token.map(|token| (token.client.clone(), token.scopes.clone()));
        state.requests.push(ReceivedDataSubjectRequest {
            method: request.method.clone(),
            path: request.uri.path().into(),
            idempotency_key: request
                .headers
                .get("idempotency-key")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned),
            token: token_id,
            body: parsed.clone(),
        });
        state.purge();
        route(
            &mut state,
            request.method,
            request.uri.path(),
            &request.headers,
            parsed,
            identity,
        )
    };
    if let Some(delay) = delay {
        tokio::time::sleep(delay).await;
    }
    response
}

fn registered_token<'a>(
    headers: &HeaderMap,
    tokens: &'a HashMap<String, Token>,
) -> Option<&'a Token> {
    let mut values = headers.get_all(header::AUTHORIZATION).iter();
    let value = values.next()?.to_str().ok()?;
    if values.next().is_some() {
        return None;
    }
    let (scheme, bearer) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    tokens.get(bearer)
}

type Reply = (Response, Option<Duration>);

fn route(
    state: &mut TargetState,
    method: Method,
    path: &str,
    headers: &HeaderMap,
    body: Option<Value>,
    identity: Option<(String, Vec<String>)>,
) -> Reply {
    if method == Method::GET && path == DISCOVERY {
        return (
            (
                [(header::CACHE_CONTROL, "public, max-age=300")],
                Json(&state.discovery),
            )
                .into_response(),
            None,
        );
    }
    let Some(suffix) = path.strip_prefix(PREFIX) else {
        return (not_found(), None);
    };
    let segments: Vec<_> = suffix.split('/').collect();
    let kind = match segments[0] {
        "erasures" => OperationKind::Erasure,
        "exports" => OperationKind::Export,
        _ => return (not_found(), None),
    };
    let Some((client, scopes)) = identity else {
        return (ApiError::unauthenticated().into_response(), None);
    };
    if client.is_empty() {
        return (
            error(StatusCode::FORBIDDEN, codes::CLIENT_NOT_ALLOWED),
            None,
        );
    }
    let scope = match kind {
        OperationKind::Erasure => &state.discovery.scopes.erasure,
        OperationKind::Export => &state.discovery.scopes.export,
    };
    if !scopes.contains(scope) {
        return (
            error(StatusCode::FORBIDDEN, codes::INSUFFICIENT_SCOPE),
            None,
        );
    }
    if method == Method::POST && segments.len() == 1 {
        return create(state, kind, client, headers, body);
    }
    if method == Method::GET
        && (segments.len() == 2
            || (kind == OperationKind::Export && segments.len() == 4 && segments[2] == "parts"))
    {
        let Ok(id) = segments[1].parse::<Uuid>() else {
            return (not_found(), None);
        };
        let Some(mut operation) = state.operations.remove(&id) else {
            return (not_found(), None);
        };
        let reply = if operation.client != client || operation.resource.kind != kind {
            (not_found(), None)
        } else {
            read(state, &mut operation, segments.get(3).copied())
        };
        state.operations.insert(id, operation);
        return reply;
    }
    (not_found(), None)
}

fn create(
    state: &mut TargetState,
    kind: OperationKind,
    client: String,
    headers: &HeaderMap,
    body: Option<Value>,
) -> Reply {
    let mut keys = headers.get_all("idempotency-key").iter();
    let key = keys.next().and_then(|value| value.to_str().ok());
    if keys.next().is_some() || validate_idempotency_key(key).is_err() {
        return (
            error(StatusCode::BAD_REQUEST, codes::IDEMPOTENCY_KEY_INVALID),
            None,
        );
    }
    let Some(request) =
        body.and_then(|body| serde_json::from_value::<DataSubjectRequest>(body).ok())
    else {
        return (
            error(StatusCode::UNPROCESSABLE_ENTITY, codes::REQUEST_INVALID),
            None,
        );
    };
    let normalized = match normalize_request(
        &request,
        &state.discovery.limits,
        &state.discovery.identifier_types,
    ) {
        Ok(normalized) => normalized,
        Err(error_value) => {
            return (
                error(StatusCode::UNPROCESSABLE_ENTITY, error_value.code()),
                None,
            );
        }
    };
    let bytes = serde_json::to_vec(&normalized).expect("normalized request serializes");
    let request_hmac = hmac::sign(&state.hmac_key, &bytes).as_ref().to_vec();
    let key = key.expect("validated key");
    if let Some(existing) = state.operations.values().find(|operation| {
        operation.client == client && operation.key == key && operation.resource.kind == kind
    }) {
        if existing.request_hmac != request_hmac {
            return (
                error(StatusCode::CONFLICT, codes::IDEMPOTENCY_CONFLICT),
                None,
            );
        }
        let (fault, delay) = transient(&existing.script);
        let response = fault.unwrap_or_else(|| operation_response(&existing.resource));
        return (response, delay);
    }
    let script = state
        .scripts
        .get(&request.subject_reference)
        .unwrap_or(&state.default_script)
        .clone();
    let (fault, delay) = transient(&script);
    if let Some(fault) = fault {
        return (fault, delay);
    }
    let outcome = lock(&script).outcome.clone();
    let mut terminal = &outcome;
    while let DataSubjectOutcome::PendingFor { then, .. } = terminal {
        terminal = then;
    }
    if matches!(
        (kind, terminal),
        (OperationKind::Erasure, DataSubjectOutcome::Export { .. })
            | (OperationKind::Export, DataSubjectOutcome::Restricted { .. })
    ) {
        return (
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "data_subject_test_script_invalid",
            ),
            delay,
        );
    }
    let resource = OperationResource {
        operation_id: Uuid::new_v4(),
        kind,
        status: OperationStatus::Pending,
        subject_reference: request.subject_reference,
        created_at: state.now(),
        completed_at: None,
        categories: Vec::new(),
        restrictions: Vec::new(),
        parts: Vec::new(),
        receipt: None,
        error: None,
    };
    let response = operation_response(&resource);
    state.operations.insert(
        resource.operation_id,
        Operation {
            client,
            key: key.into(),
            request_hmac,
            resource,
            outcome,
            script,
            parts: Vec::new(),
        },
    );
    (response, delay)
}

fn transient(script: &SharedScript) -> ReplyFault {
    let mut script = lock(script);
    let delay = script
        .faults
        .iter()
        .filter_map(|fault| match fault {
            DataSubjectFault::Delay(duration) => Some(*duration),
            _ => None,
        })
        .max();
    for fault in &mut script.faults {
        match fault {
            DataSubjectFault::ServerError { status, times } if *times > 0 => {
                *times -= 1;
                return (
                    Some(error(
                        StatusCode::from_u16(*status).expect("validated status"),
                        "server_error",
                    )),
                    delay,
                );
            }
            DataSubjectFault::RateLimited { retry_after, times } if *times > 0 => {
                *times -= 1;
                let seconds = retry_after
                    .as_secs()
                    .saturating_add(u64::from(retry_after.subsec_nanos() > 0));
                let mut response = error(StatusCode::TOO_MANY_REQUESTS, "rate_limited");
                response.headers_mut().insert(
                    header::RETRY_AFTER,
                    seconds.to_string().parse().expect("seconds header"),
                );
                return (Some(response), delay);
            }
            _ => {}
        }
    }
    (None, delay)
}

type ReplyFault = (Option<Response>, Option<Duration>);

fn read(state: &TargetState, operation: &mut Operation, part: Option<&str>) -> Reply {
    let (fault, delay) = transient(&operation.script);
    if let Some(fault) = fault {
        return (fault, delay);
    }
    if let Some(part) = part {
        if operation.resource.status != OperationStatus::Completed {
            return (error(StatusCode::CONFLICT, codes::EXPORT_NOT_READY), delay);
        }
        let completed = operation
            .resource
            .completed_at
            .expect("completed timestamp");
        if state
            .now()
            .signed_duration_since(completed)
            .to_std()
            .is_ok_and(|age| age >= state.options.export_retention)
        {
            return (error(StatusCode::GONE, codes::EXPORT_EXPIRED), delay);
        }
        let bytes = part
            .parse::<usize>()
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|index| operation.parts.get(index));
        let Some(bytes) = bytes else {
            return (not_found(), delay);
        };
        return (
            ([(header::CONTENT_TYPE, "application/json")], bytes.clone()).into_response(),
            delay,
        );
    }
    if !operation.resource.status.is_terminal() {
        if pending_poll(&mut operation.outcome) {
            return (Json(&operation.resource).into_response(), delay);
        }
        complete(state, operation);
    }
    (Json(&operation.resource).into_response(), delay)
}

fn pending_poll(outcome: &mut DataSubjectOutcome) -> bool {
    loop {
        match outcome {
            DataSubjectOutcome::PendingFor { polls, .. } if *polls > 0 => {
                *polls -= 1;
                return true;
            }
            DataSubjectOutcome::PendingFor { then, .. } => {
                *outcome = *then.clone();
            }
            _ => return false,
        }
    }
}

fn complete(state: &TargetState, operation: &mut Operation) {
    let resource = &mut operation.resource;
    resource.completed_at = Some(state.now());
    match &operation.outcome {
        DataSubjectOutcome::Completed { categories } => {
            resource.status = OperationStatus::Completed;
            resource.categories = categories.clone();
        }
        DataSubjectOutcome::NotFound => resource.status = OperationStatus::NotFound,
        DataSubjectOutcome::Restricted {
            categories,
            restrictions,
        } => {
            resource.status = OperationStatus::Restricted;
            resource.categories = categories.clone();
            resource.restrictions = restrictions.clone();
        }
        DataSubjectOutcome::Failed { code } => {
            resource.status = OperationStatus::Failed;
            resource.error = Some(OperationError { code: code.clone() });
        }
        DataSubjectOutcome::Export { parts } => {
            resource.status = OperationStatus::Completed;
            operation.parts = parts
                .iter()
                .map(|part| serde_json::to_vec(part).expect("JSON value serializes"))
                .collect();
            resource.parts = operation
                .parts
                .iter()
                .enumerate()
                .map(|(index, bytes)| ExportPart {
                    part_number: u32::try_from(index + 1).expect("validated part count"),
                    sha256: hex_digest(bytes),
                    bytes: bytes.len() as u64,
                })
                .collect();
        }
        DataSubjectOutcome::PendingFor { .. } => {
            unreachable!("pending wrappers consumed before completion")
        }
    }
    if !resource.status.has_receipt() {
        return;
    }
    let faults = lock(&operation.script).faults.clone();
    let mut payload = ReceiptPayload {
        receipt_type: RECEIPT_TYPE.into(),
        contract_version: CONTRACT_VERSION.into(),
        operation_id: resource.operation_id,
        kind: resource.kind,
        status: resource.status,
        subject_reference: resource.subject_reference.clone(),
        service: state.discovery.service.clone(),
        created_at: resource.created_at,
        completed_at: resource.completed_at.expect("terminal timestamp"),
        categories: resource.categories.clone(),
        restrictions: resource.restrictions.clone(),
        parts: resource.parts.clone(),
    };
    if faults
        .iter()
        .any(|fault| matches!(fault, DataSubjectFault::WrongSubjectReference))
    {
        let last = payload
            .subject_reference
            .pop()
            .expect("validated reference");
        payload
            .subject_reference
            .push(if last == 'x' { 'y' } else { 'x' });
    }
    let signer = if faults
        .iter()
        .any(|fault| matches!(fault, DataSubjectFault::BadSignature))
    {
        &state.unadvertised_signer
    } else {
        &state.signer
    };
    let mut receipt = signer.sign(&payload).expect("controlled receipt is valid");
    if faults
        .iter()
        .any(|fault| matches!(fault, DataSubjectFault::WrongKeyId))
    {
        receipt.signatures[0].keyid = "0".repeat(64);
    }
    if faults
        .iter()
        .any(|fault| matches!(fault, DataSubjectFault::TamperedPayload))
    {
        let mut bytes = STANDARD.decode(&receipt.payload).expect("signer base64");
        bytes.push(b' ');
        receipt.payload = STANDARD.encode(bytes);
    }
    if faults
        .iter()
        .any(|fault| matches!(fault, DataSubjectFault::PartDigestMismatch))
    {
        for bytes in &mut operation.parts {
            let mut changed = vec![b' '; bytes.len()];
            changed[0] = if bytes == b"0" { b'1' } else { b'0' };
            *bytes = changed;
        }
    }
    resource.receipt = Some(receipt);
}

fn hex_digest(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn operation_response(resource: &OperationResource) -> Response {
    let status = if resource.status.is_terminal() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    (status, Json(resource)).into_response()
}

fn error(status: StatusCode, code: &str) -> Response {
    ApiError::new(
        status,
        code,
        "The data subject request could not be processed",
    )
    .into_response()
}

fn not_found() -> Response {
    error(StatusCode::NOT_FOUND, codes::OPERATION_NOT_FOUND)
}
