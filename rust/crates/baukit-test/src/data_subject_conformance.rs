use std::time::Duration;

use aws_lc_rs::digest::{SHA256, digest};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{HeaderMap, Request, StatusCode, header},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_data_subject::{
    CONTRACT_VERSION, DataSubjectRequest, DiscoveryDocument, Identifier, IdentifierType,
    OperationKind, OperationResource, OperationStatus, ReceiptVerifier, Scopes, codes,
    normalize_request,
};
use serde_json::{Value, json};
use tower::ServiceExt as _;
use uuid::Uuid;

const DISCOVERY: &str = "/.well-known/baukit/data-subject";
const DEADLINE: Duration = Duration::from_secs(30);

/// HTTP connection used by the service conformance check.
#[derive(Clone, Copy)]
pub enum DataSubjectEndpoint<'a> {
    /// Exercise the complete product router, including authentication middleware.
    Router(&'a Router),
    /// Exercise an already listening service at this origin or path prefix.
    BaseUrl(&'a str),
}

/// Machine identity requested from the product's token issuer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSubjectTestClient {
    /// Primary allowed OAuth client.
    Allowed,
    /// A different allowed OAuth client for ownership checks.
    OtherAllowed,
    /// An authenticated OAuth client outside the allowlist.
    Disallowed,
    /// An authenticated API token, even if it has the requested grants.
    ApiToken,
}

/// Isolated subject fixture requested for one advertised operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataSubjectTestSubject {
    /// A subject with erasable data and at least one downloadable export part.
    Matched,
    /// Identifiers that match no subject.
    Unmatched,
    /// A subject with legal retention obligations, if the product supports them.
    Restricted,
}

/// Product setup and worker controls. All service observations use HTTP.
///
/// Use a fresh fixture for each check. Tokens and lookup values must stay out of
/// diagnostics. Lookup values should be distinctive and absent from export content
/// so the check can detect accidental disclosure in every response body.
#[async_trait::async_trait]
pub trait DataSubjectConformanceAdapter: Send {
    /// Adapter failure type. Its contents are never included in check errors.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Mints verified OAuth tokens with exactly these scopes, or API tokens
    /// carrying at least these grants.
    async fn mint_token(
        &mut self,
        client: DataSubjectTestClient,
        scopes: &[&str],
    ) -> Result<String, Self::Error>;

    /// Seeds fresh, independent identifiers accepted by discovery's types and limits.
    ///
    /// `reference` is the opaque reference the check will submit. Return `None`
    /// only for unsupported restrictions. Matched export fixtures must have parts.
    /// Calls for separate kinds must not reuse subjects erased by earlier calls.
    async fn seed_subject(
        &mut self,
        kind: OperationKind,
        case: DataSubjectTestSubject,
        reference: &str,
    ) -> Result<Option<Vec<Identifier>>, Self::Error>;

    /// Runs pending work to completion before returning, without polling sleeps
    /// in the check. Include all required identity or external-work test workers.
    async fn run_pending_work(&mut self) -> Result<(), Self::Error>;
}

/// Failed contract requirement. No variant retains response bodies or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DataSubjectConformanceError {
    /// Fixture setup, token issuance or worker driving failed.
    #[error("data subject conformance adapter failed")]
    Adapter,
    /// An HTTP request failed, timed out or exceeded its response bound.
    #[error("data subject conformance HTTP exchange failed")]
    Transport,
    /// Discovery has an invalid shape, service, types, operations or bounds.
    #[error("data subject discovery shape is invalid")]
    DiscoveryShape,
    /// Discovery advertises a version other than v1.
    #[error("data subject discovery contract version is invalid")]
    ContractVersion,
    /// Discovery is not publicly cacheable for five minutes.
    #[error("data subject discovery cache control is invalid")]
    CacheControl,
    /// Discovery keys are invalid or do not include exactly one current key.
    #[error("data subject discovery receipt keys are invalid")]
    ReceiptKeys,
    /// Discovery scopes differ from the v1 scopes.
    #[error("data subject discovery scopes are invalid")]
    Scopes,
    /// An advertised operation route is absent.
    #[error("data subject advertised route is absent")]
    Routes,
    /// A protected route did not return 401 without a principal.
    #[error("data subject route accepted a missing principal")]
    Unauthorized,
    /// A disallowed client or API token did not receive the required 403 code.
    #[error("data subject client allowlist is not enforced")]
    ClientNotAllowed,
    /// A protected operation did not enforce its own scope.
    #[error("data subject operation scope is not enforced")]
    InsufficientScope,
    /// Missing, malformed or repeated idempotency keys did not return 400.
    #[error("data subject idempotency key validation is invalid")]
    IdempotencyKey,
    /// An invalid request body did not return 422 with the required code.
    #[error("data subject request body validation is invalid")]
    InvalidBody,
    /// An unadvertised identifier type did not return the required 422 code.
    #[error("data subject identifier type allowlist is not enforced")]
    UnsupportedIdentifierType,
    /// Advertised identifier count or byte limits were not enforced.
    #[error("data subject identifier limits are not enforced")]
    Limits,
    /// A new POST did not return a pending 202 resource.
    #[error("data subject operation acceptance is invalid")]
    Acceptance,
    /// A replay changed the operation or returned an invalid status.
    #[error("data subject operation replay is invalid")]
    Replay,
    /// A different normalized request under the same key did not return 409.
    #[error("data subject idempotency conflict is not enforced")]
    Conflict,
    /// Another client or kind could see an operation or its parts.
    #[error("data subject operation isolation is not enforced")]
    Isolation,
    /// The terminal receipt is missing or fails discovery-based verification.
    #[error("data subject terminal receipt does not verify")]
    Receipt,
    /// Receipt fields differ from discovery or the operation resource.
    #[error("data subject receipt does not match its operation")]
    ReceiptFields,
    /// The worker did not reach the fixture's expected terminal result.
    #[error("data subject operation did not complete as expected")]
    Completion,
    /// Unmatched identifiers did not produce a signed empty notFound result.
    #[error("data subject unmatched identifiers did not produce notFound")]
    NotFound,
    /// A restricted fixture did not describe retained categories and obligations.
    #[error("data subject restricted erasure result is invalid")]
    Restricted,
    /// A pending export part did not return the required 409 code.
    #[error("data subject pending export part was not rejected")]
    ExportNotReady,
    /// Export bytes, JSON, part numbers, digest, length or bound are invalid.
    #[error("data subject export part does not match its signed metadata")]
    ExportPart,
    /// A submitted lookup value appeared in a response or decoded receipt.
    #[error("data subject response disclosed a submitted identifier")]
    IdentifierLeak,
}

use DataSubjectConformanceError as Failure;

/// Checks each advertised v1 operation through HTTP, failing at the first violation.
///
/// The adapter supplies isolated fixtures and runs workers. Unadvertised kinds
/// are skipped. Responses and decoded receipts are checked for submitted lookup
/// values; failures disclose only a typed requirement. This consumes fixture data.
pub async fn check_data_subject_conformance<A: DataSubjectConformanceAdapter>(
    endpoint: DataSubjectEndpoint<'_>,
    adapter: &mut A,
) -> Result<(), DataSubjectConformanceError> {
    let mut http = Http {
        endpoint,
        client: reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(DEADLINE)
            .build()
            .map_err(|_| Failure::Transport)?,
        identifiers: Vec::new(),
        body_limit: 1_048_576,
    };
    let discovery = discovery(&mut http).await?;
    let verifier = ReceiptVerifier::from_discovery(&discovery).map_err(|_| Failure::ReceiptKeys)?;
    http.body_limit = http.body_limit.max(
        usize::try_from(discovery.limits.max_export_part_bytes)
            .map_err(|_| Failure::DiscoveryShape)?,
    );
    let scopes = ["data-subject.erase", "data-subject.export"];
    let token = adapter
        .mint_token(DataSubjectTestClient::Allowed, &scopes)
        .await
        .map_err(|_| Failure::Adapter)?;
    let other = adapter
        .mint_token(DataSubjectTestClient::OtherAllowed, &scopes)
        .await
        .map_err(|_| Failure::Adapter)?;
    for &kind in &discovery.operations {
        let reference = Uuid::new_v4().to_string();
        let identifiers = adapter
            .seed_subject(kind, DataSubjectTestSubject::Matched, &reference)
            .await
            .map_err(|_| Failure::Adapter)?
            .ok_or(Failure::Adapter)?;
        let request = DataSubjectRequest {
            subject_reference: reference,
            identifiers,
        };
        normalize_request(&request, &discovery.limits, &discovery.identifier_types)
            .map_err(|_| Failure::Adapter)?;
        http.remember(&serde_json::to_value(&request).map_err(|_| Failure::Adapter)?);
        check_auth(&mut http, adapter, kind, &request).await?;
        check_validation(&mut http, &discovery, kind, &token, &request).await?;
        exercise(
            &mut http,
            adapter,
            Exercise {
                discovery: &discovery,
                verifier: &verifier,
                token: &token,
                other: &other,
                kind,
                request: &request,
                expected: OperationStatus::Completed,
            },
        )
        .await?;
        for (case, expected) in [
            (DataSubjectTestSubject::Unmatched, OperationStatus::NotFound),
            (
                DataSubjectTestSubject::Restricted,
                OperationStatus::Restricted,
            ),
        ] {
            if case == DataSubjectTestSubject::Restricted && kind != OperationKind::Erasure {
                continue;
            }
            let reference = Uuid::new_v4().to_string();
            let identifiers = adapter
                .seed_subject(kind, case, &reference)
                .await
                .map_err(|_| Failure::Adapter)?;
            let Some(identifiers) = identifiers else {
                if case == DataSubjectTestSubject::Restricted {
                    continue;
                }
                return Err(Failure::Adapter);
            };
            let request = DataSubjectRequest {
                subject_reference: reference,
                identifiers,
            };
            normalize_request(&request, &discovery.limits, &discovery.identifier_types)
                .map_err(|_| Failure::Adapter)?;
            exercise(
                &mut http,
                adapter,
                Exercise {
                    discovery: &discovery,
                    verifier: &verifier,
                    token: &token,
                    other: &other,
                    kind,
                    request: &request,
                    expected,
                },
            )
            .await?;
        }
    }
    Ok(())
}

/// Panics with the failed requirement if service conformance fails.
///
/// # Panics
/// Panics without printing tokens, lookup values, adapter errors or response bodies.
pub async fn assert_data_subject_conformance<A: DataSubjectConformanceAdapter>(
    endpoint: DataSubjectEndpoint<'_>,
    adapter: &mut A,
) {
    if let Err(error) = check_data_subject_conformance(endpoint, adapter).await {
        panic!("{error}");
    }
}

struct Http<'a> {
    endpoint: DataSubjectEndpoint<'a>,
    client: reqwest::Client,
    identifiers: Vec<String>,
    body_limit: usize,
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self, failure: Failure) -> Result<Value, Failure> {
        serde_json::from_slice(&self.bytes).map_err(|_| failure)
    }

    fn operation(&self, failure: Failure) -> Result<OperationResource, Failure> {
        serde_json::from_slice(&self.bytes).map_err(|_| failure)
    }

    fn error(&self, status: u16, code: &str, failure: Failure) -> Result<(), Failure> {
        let value = self.json(failure)?;
        require(
            self.status.as_u16() == status
                && value["error"]["code"] == code
                && value["error"]["message"].is_string()
                && value["error"]["requestId"].is_string()
                && value["error"]["details"].is_object(),
            failure,
        )
    }
}

impl Http<'_> {
    fn remember(&mut self, body: &Value) {
        if let Some(identifiers) = body["identifiers"].as_array() {
            for identifier in identifiers {
                if let Some(value) = identifier["value"].as_str()
                    && !value.trim().is_empty()
                {
                    self.identifiers.push(value.to_owned());
                    self.identifiers.push(value.trim().to_owned());
                    if identifier["type"] == "email" {
                        self.identifiers.push(value.trim().to_lowercase());
                    }
                }
            }
        }
    }

    fn private(&self, bytes: &[u8]) -> Result<(), Failure> {
        let text = String::from_utf8_lossy(bytes);
        require(
            !self.identifiers.iter().any(|value| text.contains(value)),
            Failure::IdentifierLeak,
        )?;
        if let Ok(json) = serde_json::from_slice::<Value>(bytes) {
            self.private_json(&json)?;
            if let Some(payload) = json["receipt"]["payload"].as_str()
                && let Ok(decoded) = STANDARD.decode(payload)
            {
                self.private(&decoded)?;
            }
        }
        Ok(())
    }

    fn private_json(&self, value: &Value) -> Result<(), Failure> {
        match value {
            Value::String(text) => require(
                !self.identifiers.iter().any(|value| text.contains(value)),
                Failure::IdentifierLeak,
            ),
            Value::Array(values) => {
                for value in values {
                    self.private_json(value)?;
                }
                Ok(())
            }
            Value::Object(values) => {
                for (key, value) in values {
                    self.private_json(&Value::String(key.clone()))?;
                    self.private_json(value)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    async fn send(
        &mut self,
        method: &str,
        path: &str,
        token: Option<&str>,
        keys: &[&str],
        body: Option<Value>,
    ) -> Result<Reply, Failure> {
        if let Some(body) = &body {
            self.remember(body);
        }
        let bytes = body
            .map(|body| serde_json::to_vec(&body))
            .transpose()
            .map_err(|_| Failure::Adapter)?
            .unwrap_or_default();
        self.send_bytes(method, path, token, keys, bytes).await
    }

    async fn send_bytes(
        &self,
        method: &str,
        path: &str,
        token: Option<&str>,
        keys: &[&str],
        bytes: Vec<u8>,
    ) -> Result<Reply, Failure> {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(token) = token {
            request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        for key in keys {
            request = request.header("idempotency-key", *key);
        }
        let request = request
            .body(Body::from(bytes.clone()))
            .map_err(|_| Failure::Adapter)?;
        let reply = tokio::time::timeout(DEADLINE, async {
            match self.endpoint {
                DataSubjectEndpoint::Router(router) => {
                    let response = router
                        .clone()
                        .oneshot(request)
                        .await
                        .map_err(|_| Failure::Transport)?;
                    let (parts, body) = response.into_parts();
                    let bytes = to_bytes(body, self.body_limit)
                        .await
                        .map_err(|_| Failure::Transport)?
                        .to_vec();
                    Ok(Reply {
                        status: parts.status,
                        headers: parts.headers,
                        bytes,
                    })
                }
                DataSubjectEndpoint::BaseUrl(base) => {
                    let mut response = self
                        .client
                        .request(
                            request.method().clone(),
                            format!("{}{path}", base.trim_end_matches('/')),
                        )
                        .headers(request.headers().clone())
                        .body(bytes)
                        .send()
                        .await
                        .map_err(|_| Failure::Transport)?;
                    let status = response.status();
                    let headers = response.headers().clone();
                    let mut bytes = Vec::new();
                    while let Some(chunk) =
                        response.chunk().await.map_err(|_| Failure::Transport)?
                    {
                        require(
                            chunk.len() <= self.body_limit.saturating_sub(bytes.len()),
                            Failure::Transport,
                        )?;
                        bytes.extend_from_slice(&chunk);
                    }
                    Ok(Reply {
                        status,
                        headers,
                        bytes,
                    })
                }
            }
        })
        .await
        .map_err(|_| Failure::Transport)??;
        self.private(&reply.bytes)?;
        Ok(reply)
    }
}

fn require(condition: bool, failure: Failure) -> Result<(), Failure> {
    if condition { Ok(()) } else { Err(failure) }
}

fn route(kind: OperationKind) -> &'static str {
    match kind {
        OperationKind::Erasure => "/api/v1/data-subject/erasures",
        OperationKind::Export => "/api/v1/data-subject/exports",
    }
}

async fn discovery(http: &mut Http<'_>) -> Result<DiscoveryDocument, Failure> {
    let reply = http.send("GET", DISCOVERY, None, &[], None).await?;
    require(
        reply.status == StatusCode::OK && is_json(&reply.headers),
        Failure::DiscoveryShape,
    )?;
    let value = reply.json(Failure::DiscoveryShape)?;
    require(
        value["contractVersion"] == CONTRACT_VERSION,
        Failure::ContractVersion,
    )?;
    let discovery: DiscoveryDocument =
        serde_json::from_value(value).map_err(|_| Failure::DiscoveryShape)?;
    require(valid_discovery_cache(&reply.headers), Failure::CacheControl)?;
    require(discovery.scopes == Scopes::default(), Failure::Scopes)?;
    require(
        !discovery.service.name.trim().is_empty()
            && !discovery.service.version.trim().is_empty()
            && !discovery.operations.is_empty()
            && discovery
                .operations
                .iter()
                .enumerate()
                .all(|(i, kind)| !discovery.operations[..i].contains(kind))
            && !discovery.identifier_types.is_empty()
            && discovery
                .identifier_types
                .iter()
                .enumerate()
                .all(|(i, kind)| !discovery.identifier_types[..i].contains(kind))
            && discovery.limits.max_identifiers > 0
            && discovery.limits.max_identifier_length > 0
            && discovery.limits.max_export_part_bytes > 0,
        Failure::DiscoveryShape,
    )?;
    Ok(discovery)
}

fn valid_discovery_cache(headers: &HeaderMap) -> bool {
    let mut max_age = None;
    for header in headers.get_all(header::CACHE_CONTROL) {
        let Ok(value) = header.to_str() else {
            return false;
        };
        for directive in value.split(',') {
            let (name, value) = directive
                .trim()
                .split_once('=')
                .map_or((directive.trim(), None), |(name, value)| {
                    (name.trim(), Some(value.trim().trim_matches('"')))
                });
            if ["no-store", "no-cache", "private"]
                .iter()
                .any(|v| name.eq_ignore_ascii_case(v))
            {
                return false;
            }
            if name.eq_ignore_ascii_case("max-age") {
                if max_age.is_some() {
                    return false;
                }
                let Some(parsed) = value.and_then(|value| value.parse::<u64>().ok()) else {
                    return false;
                };
                max_age = Some(parsed);
            }
            if name.eq_ignore_ascii_case("s-maxage") && value != Some("300") {
                return false;
            }
        }
    }
    max_age == Some(300)
}

fn is_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';')
                .next()
                .is_some_and(|v| v.trim() == "application/json")
        })
}

async fn check_auth<A: DataSubjectConformanceAdapter>(
    http: &mut Http<'_>,
    adapter: &mut A,
    kind: OperationKind,
    request: &DataSubjectRequest,
) -> Result<(), Failure> {
    let path = route(kind);
    let scope = match kind {
        OperationKind::Erasure => "data-subject.erase",
        OperationKind::Export => "data-subject.export",
    };
    let other_scope = match kind {
        OperationKind::Erasure => "data-subject.export",
        OperationKind::Export => "data-subject.erase",
    };
    let id = Uuid::new_v4();
    let mut routes = vec![("POST", path.to_owned()), ("GET", format!("{path}/{id}"))];
    if kind == OperationKind::Export {
        routes.push(("GET", format!("{path}/{id}/parts/1")));
    }
    let body = serde_json::to_value(request).map_err(|_| Failure::Adapter)?;
    for (method, path) in routes {
        let reply = http
            .send(
                method,
                &path,
                None,
                &["conformance-key-01"],
                Some(body.clone()),
            )
            .await?;
        require(
            !matches!(
                reply.status,
                StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ),
            Failure::Routes,
        )?;
        reply.error(401, "unauthenticated", Failure::Unauthorized)?;
        for client in [
            DataSubjectTestClient::Disallowed,
            DataSubjectTestClient::ApiToken,
        ] {
            let token = adapter
                .mint_token(client, &[scope])
                .await
                .map_err(|_| Failure::Adapter)?;
            http.send(
                method,
                &path,
                Some(&token),
                &["conformance-key-01"],
                Some(body.clone()),
            )
            .await?
            .error(403, codes::CLIENT_NOT_ALLOWED, Failure::ClientNotAllowed)?;
        }
        for scopes in [vec![], vec![other_scope]] {
            let token = adapter
                .mint_token(DataSubjectTestClient::Allowed, &scopes)
                .await
                .map_err(|_| Failure::Adapter)?;
            http.send(
                method,
                &path,
                Some(&token),
                &["conformance-key-01"],
                Some(body.clone()),
            )
            .await?
            .error(403, codes::INSUFFICIENT_SCOPE, Failure::InsufficientScope)?;
        }
    }
    Ok(())
}

async fn check_validation(
    http: &mut Http<'_>,
    discovery: &DiscoveryDocument,
    kind: OperationKind,
    token: &str,
    request: &DataSubjectRequest,
) -> Result<(), Failure> {
    let path = route(kind);
    let body = serde_json::to_value(request).map_err(|_| Failure::Adapter)?;
    let long = "k".repeat(129);
    for keys in [
        vec![],
        vec!["k"; 1],
        vec![long.as_str()],
        vec!["conformance-key-01", "conformance-key-01"],
        vec!["invalid key with spaces"],
    ] {
        http.send("POST", path, Some(token), &keys, Some(body.clone()))
            .await?
            .error(400, codes::IDEMPOTENCY_KEY_INVALID, Failure::IdempotencyKey)?;
    }
    let mut invalid = vec![
        Value::Null,
        json!({}),
        json!({"subjectReference": request.subject_reference, "identifiers": []}),
    ];
    for reference in [
        "r".repeat(15),
        "r".repeat(129),
        "reference with spaces".into(),
        "référence-non-ascii".into(),
    ] {
        let mut changed = body.clone();
        changed["subjectReference"] = json!(reference);
        invalid.push(changed);
    }
    for (kind, value) in [
        ("x-Bad", "invalid-type-value"),
        (request.identifiers[0].identifier_type.as_str(), "   "),
    ] {
        let mut changed = body.clone();
        changed["identifiers"] = json!([{"type":kind,"value":value}]);
        invalid.push(changed);
    }
    if discovery.identifier_types.contains(&IdentifierType::Phone) {
        let mut changed = body.clone();
        changed["identifiers"] = json!([{"type":"phone","value":"+1234invalid5678"}]);
        invalid.push(changed);
    }
    http.send_bytes(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        b"{broken".to_vec(),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::InvalidBody)?;
    for body in invalid {
        http.send(
            "POST",
            path,
            Some(token),
            &["conformance-key-01"],
            Some(body),
        )
        .await?
        .error(422, codes::REQUEST_INVALID, Failure::InvalidBody)?;
    }
    let mut unsupported = "x-conformance-unsupported".to_owned();
    while discovery
        .identifier_types
        .iter()
        .any(|kind| kind.as_str() == unsupported)
    {
        unsupported.push_str("-other");
    }
    let mut changed = body.clone();
    changed["identifiers"][0]["type"] = json!(unsupported);
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(
        422,
        codes::IDENTIFIER_TYPE_UNSUPPORTED,
        Failure::UnsupportedIdentifierType,
    )?;
    let mut changed = body.clone();
    changed["identifiers"] = json!(vec![
        body["identifiers"][0].clone();
        discovery
            .limits
            .max_identifiers
            .checked_add(1)
            .ok_or(Failure::DiscoveryShape)?
    ]);
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::Limits)?;
    let mut changed = body;
    changed["identifiers"][0]["value"] = json!(
        "z".repeat(
            discovery
                .limits
                .max_identifier_length
                .checked_add(1)
                .ok_or(Failure::DiscoveryShape)?
        )
    );
    http.send(
        "POST",
        path,
        Some(token),
        &["conformance-key-01"],
        Some(changed),
    )
    .await?
    .error(422, codes::REQUEST_INVALID, Failure::Limits)?;
    Ok(())
}

struct Exercise<'a> {
    discovery: &'a DiscoveryDocument,
    verifier: &'a ReceiptVerifier,
    token: &'a str,
    other: &'a str,
    kind: OperationKind,
    request: &'a DataSubjectRequest,
    expected: OperationStatus,
}

async fn exercise<A: DataSubjectConformanceAdapter>(
    http: &mut Http<'_>,
    adapter: &mut A,
    case: Exercise<'_>,
) -> Result<(), Failure> {
    let Exercise {
        discovery,
        verifier,
        token,
        other,
        kind,
        request,
        expected,
    } = case;
    let path = route(kind);
    let key = Uuid::new_v4().to_string();
    let body = serde_json::to_value(request).map_err(|_| Failure::Adapter)?;
    let reply = http
        .send("POST", path, Some(token), &[&key], Some(body.clone()))
        .await?;
    require(
        !matches!(
            reply.status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ),
        Failure::Routes,
    )?;
    require(
        reply.status == StatusCode::ACCEPTED && is_json(&reply.headers),
        Failure::Acceptance,
    )?;
    let pending = reply.operation(Failure::Acceptance)?;
    require(
        pending.kind == kind
            && pending.status == OperationStatus::Pending
            && pending.subject_reference == request.subject_reference
            && pending.completed_at.is_none()
            && pending.receipt.is_none()
            && pending.error.is_none()
            && pending.parts.is_empty(),
        Failure::Acceptance,
    )?;
    let status_path = format!("{path}/{}", pending.operation_id);
    if kind == OperationKind::Export {
        http.send(
            "GET",
            &format!("{status_path}/parts/1"),
            Some(token),
            &[],
            None,
        )
        .await?
        .error(409, codes::EXPORT_NOT_READY, Failure::ExportNotReady)?;
    }
    let replay = http
        .send("POST", path, Some(token), &[&key], Some(body.clone()))
        .await?;
    let first_replay = check_replay(&replay, &pending, discovery, verifier)?;
    let mut normalized_replay = body.clone();
    for identifier in normalized_replay["identifiers"]
        .as_array_mut()
        .ok_or(Failure::Adapter)?
    {
        if identifier["type"] != "phone" {
            let value = identifier["value"].as_str().ok_or(Failure::Adapter)?;
            if value.len() + 2 <= discovery.limits.max_identifier_length {
                identifier["value"] = json!(format!(" {value} "));
            }
        }
    }
    let replay = http
        .send("POST", path, Some(token), &[&key], Some(normalized_replay))
        .await?;
    let normalized_replay = check_replay(&replay, &pending, discovery, verifier)?;
    let mut conflict = body.clone();
    conflict["subjectReference"] = json!(Uuid::new_v4().to_string());
    http.send("POST", path, Some(token), &[&key], Some(conflict))
        .await?
        .error(409, codes::IDEMPOTENCY_CONFLICT, Failure::Conflict)?;
    check_isolation(http, discovery, kind, pending.operation_id, token, other).await?;
    tokio::time::timeout(DEADLINE, adapter.run_pending_work())
        .await
        .map_err(|_| Failure::Adapter)?
        .map_err(|_| Failure::Adapter)?;
    let reply = http
        .send("GET", &status_path, Some(token), &[], None)
        .await?;
    require(
        !matches!(
            reply.status,
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
        ),
        Failure::Routes,
    )?;
    require(
        reply.status == StatusCode::OK && is_json(&reply.headers),
        Failure::Completion,
    )?;
    let done = reply.operation(Failure::Completion)?;
    let state_failure = match expected {
        OperationStatus::NotFound => Failure::NotFound,
        OperationStatus::Restricted => Failure::Restricted,
        _ => Failure::Completion,
    };
    require(
        done.status == expected
            && done.operation_id == pending.operation_id
            && done.kind == kind
            && done.subject_reference == request.subject_reference
            && done.created_at == pending.created_at
            && done.completed_at.is_some()
            && done.error.is_none(),
        state_failure,
    )?;
    verify_terminal(&done, discovery, verifier)?;
    check_isolation(http, discovery, kind, done.operation_id, token, other).await?;
    if kind == OperationKind::Export && expected == OperationStatus::NotFound {
        http.send(
            "GET",
            &format!("{status_path}/parts/1"),
            Some(token),
            &[],
            None,
        )
        .await?
        .error(404, codes::OPERATION_NOT_FOUND, Failure::NotFound)?;
    }
    for replay in [first_replay, normalized_replay] {
        if replay.status.is_terminal() {
            require(replay == done, Failure::Replay)?;
        }
    }
    if expected == OperationStatus::NotFound {
        require(
            done.categories.is_empty() && done.restrictions.is_empty() && done.parts.is_empty(),
            Failure::NotFound,
        )?;
    }
    if expected == OperationStatus::Restricted {
        require(
            !done.restrictions.is_empty()
                && done.restrictions.iter().all(|restriction| {
                    !restriction.reason.is_empty()
                        && !restriction.legal_basis.is_empty()
                        && done.categories.iter().any(|category| {
                            category.category == restriction.category
                                && category.action == baukit_data_subject::CategoryAction::Retained
                        })
                }),
            Failure::Restricted,
        )?;
    }
    if expected == OperationStatus::Completed {
        require(done.restrictions.is_empty(), Failure::Completion)?;
    }
    if kind == OperationKind::Erasure {
        require(done.parts.is_empty(), Failure::ExportPart)?;
    }
    if kind == OperationKind::Export && expected == OperationStatus::Completed {
        require(!done.parts.is_empty(), Failure::ExportPart)?;
        for (index, part) in done.parts.iter().enumerate() {
            require(
                part.part_number as usize == index + 1
                    && part.bytes <= discovery.limits.max_export_part_bytes,
                Failure::ExportPart,
            )?;
            let part_path = format!("{status_path}/parts/{}", part.part_number);
            let reply = http.send("GET", &part_path, Some(token), &[], None).await?;
            let hash: String = digest(&SHA256, &reply.bytes)
                .as_ref()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            require(
                reply.status == StatusCode::OK
                    && is_json(&reply.headers)
                    && reply.bytes.len() as u64 == part.bytes
                    && hash == part.sha256,
                Failure::ExportPart,
            )?;
            reply.json(Failure::ExportPart)?;
            http.send("GET", &part_path, Some(other), &[], None)
                .await?
                .error(404, codes::OPERATION_NOT_FOUND, Failure::Isolation)?;
        }
    }
    let replay = http
        .send("POST", path, Some(token), &[&key], Some(body))
        .await?;
    require(
        replay.status == StatusCode::OK && replay.operation(Failure::Replay)? == done,
        Failure::Replay,
    )?;
    Ok(())
}

fn verify_terminal(
    done: &OperationResource,
    discovery: &DiscoveryDocument,
    verifier: &ReceiptVerifier,
) -> Result<(), Failure> {
    let payload = verifier
        .verify(done.receipt.as_ref().ok_or(Failure::Receipt)?)
        .map_err(|_| Failure::Receipt)?;
    require(
        payload.operation_id == done.operation_id
            && payload.kind == done.kind
            && payload.status == done.status
            && payload.subject_reference == done.subject_reference
            && payload.service == discovery.service
            && payload.created_at == done.created_at
            && Some(payload.completed_at) == done.completed_at
            && payload.categories == done.categories
            && payload.restrictions == done.restrictions
            && payload.parts == done.parts,
        Failure::ReceiptFields,
    )?;
    Ok(())
}

fn check_replay(
    reply: &Reply,
    pending: &OperationResource,
    discovery: &DiscoveryDocument,
    verifier: &ReceiptVerifier,
) -> Result<OperationResource, Failure> {
    let operation = reply.operation(Failure::Replay)?;
    require(
        operation.operation_id == pending.operation_id
            && operation.kind == pending.kind
            && operation.subject_reference == pending.subject_reference
            && operation.created_at == pending.created_at,
        Failure::Replay,
    )?;
    if operation.status.is_terminal() {
        require(reply.status == StatusCode::OK, Failure::Replay)?;
        verify_terminal(&operation, discovery, verifier)?;
    } else {
        require(
            reply.status == StatusCode::ACCEPTED && operation == *pending,
            Failure::Replay,
        )?;
    }
    Ok(operation)
}

async fn check_isolation(
    http: &mut Http<'_>,
    discovery: &DiscoveryDocument,
    kind: OperationKind,
    id: Uuid,
    token: &str,
    other: &str,
) -> Result<(), Failure> {
    http.send(
        "GET",
        &format!("{}/{}", route(kind), id),
        Some(other),
        &[],
        None,
    )
    .await?
    .error(404, codes::OPERATION_NOT_FOUND, Failure::Isolation)?;
    for &other_kind in &discovery.operations {
        if other_kind != kind {
            http.send(
                "GET",
                &format!("{}/{}", route(other_kind), id),
                Some(token),
                &[],
                None,
            )
            .await?
            .error(404, codes::OPERATION_NOT_FOUND, Failure::Isolation)?;
        }
    }
    if discovery.operations.contains(&OperationKind::Export) {
        let part_path = format!("{}/{}/parts/1", route(OperationKind::Export), id);
        http.send("GET", &part_path, Some(other), &[], None)
            .await?
            .error(404, codes::OPERATION_NOT_FOUND, Failure::Isolation)?;
        if kind == OperationKind::Erasure {
            http.send("GET", &part_path, Some(token), &[], None)
                .await?
                .error(404, codes::OPERATION_NOT_FOUND, Failure::Isolation)?;
        }
    }
    Ok(())
}
