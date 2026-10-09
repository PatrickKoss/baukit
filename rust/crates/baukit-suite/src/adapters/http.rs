pub use super::http_support::SuiteHttpState;
use super::http_support::*;
use crate::domain::*;
use crate::services::{
    AuthorizationDeny, AuthorizationRequest, SignedHeaders, SuiteApi, SuiteLinkView, SuiteUser,
};
use axum::{
    Json,
    body::Bytes,
    extract::{DefaultBodyLimit, Query, State, rejection::BytesRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use baukit_http::ApiPath;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteRewardMode {
    Native,
    SourceXp,
    Off,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteLinkStatus {
    Active,
    NeedsAttention,
    Revoked,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteLinkRole {
    Initiator,
    Authorizer,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteDeliveryHealth {
    Healthy,
    Degraded,
    NeedsAttention,
    Disabled,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteDeliveryStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
enum SuiteDeliveryFailureCode {
    SuiteUnauthorized,
    SuiteLinkRevoked,
    SuiteRejected,
    SuiteLinkDisabled,
    SuitePeerUnreachable,
    SuitePayloadInvalid,
    SuiteJobTypeUnsupported,
    InternalError,
    SuiteLinkInactive,
    NotFound,
}
impl From<&str> for SuiteDeliveryFailureCode {
    fn from(code: &str) -> Self {
        match code {
            "suite_unauthorized" => Self::SuiteUnauthorized,
            "suite_link_revoked" => Self::SuiteLinkRevoked,
            "suite_rejected" => Self::SuiteRejected,
            "suite_link_disabled" => Self::SuiteLinkDisabled,
            "suite_peer_unreachable" => Self::SuitePeerUnreachable,
            "suite_payload_invalid" => Self::SuitePayloadInvalid,
            "suite_job_type_unsupported" => Self::SuiteJobTypeUnsupported,
            "suite_link_inactive" => Self::SuiteLinkInactive,
            "not_found" => Self::NotFound,
            "internal_error" => Self::InternalError,
            _ => {
                tracing::warn!(stored_code = code, "unknown suite delivery failure code");
                Self::InternalError
            }
        }
    }
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuiteLinkResponse {
    id: Uuid,
    peer_app: String,
    #[schema(value_type = SuiteLinkRole)]
    role: LinkRole,
    remote_link_id: Uuid,
    remote_display_name: Option<String>,
    #[schema(value_type = SuiteLinkStatus)]
    status: LinkStatus,
    sends: Vec<String>,
    receives: Vec<String>,
    share_xp: bool,
    #[schema(value_type = SuiteRewardMode)]
    reward_mode: RewardMode,
    #[schema(value_type = SuiteDeliveryHealth)]
    delivery_health: DeliveryHealth,
    consecutive_failures: u32,
    last_delivery_at: Option<DateTime<Utc>>,
    last_failure_at: Option<DateTime<Utc>>,
    last_failure_code: Option<SuiteDeliveryFailureCode>,
    last_received_at: Option<DateTime<Utc>>,
    replay_earliest: NaiveDate,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
impl From<SuiteLinkView> for SuiteLinkResponse {
    fn from(view: SuiteLinkView) -> Self {
        let l = view.link;
        Self {
            id: l.id,
            peer_app: l.peer_app,
            role: l.role,
            remote_link_id: l.remote_link_id,
            remote_display_name: l.remote_display_name,
            status: l.status,
            sends: l.sends,
            receives: l.receives,
            share_xp: l.share_xp,
            reward_mode: l.reward_mode,
            delivery_health: l.delivery_health,
            consecutive_failures: l.consecutive_failures,
            last_delivery_at: l.last_delivery_at,
            last_failure_at: l.last_failure_at,
            last_failure_code: l.last_failure_code.as_deref().map(Into::into),
            last_received_at: l.last_received_at,
            replay_earliest: view.replay_earliest,
            created_at: l.created_at,
            updated_at: l.updated_at,
        }
    }
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct PeerResponse {
    id: String,
    display_name: String,
    scheme: String,
    web_url: String,
    share_xp_available: bool,
    sends: Vec<String>,
    receives: Vec<String>,
    #[schema(value_type = Vec<SuiteRewardMode>)]
    reward_modes: Vec<RewardMode>,
    link: Option<Uuid>,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct StartResponse {
    request_id: Uuid,
    authorize_url: String,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct RedirectResponse {
    redirect_url: String,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct PreviewResponse {
    peer: String,
    authorizer_sends: Vec<String>,
    initiator_sends: Vec<String>,
    existing_link: bool,
    auto_approve: bool,
    hint_mismatch: bool,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct ExchangeResponseDto {
    link_id: Uuid,
    subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    suite_subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    authorizer_sends: Vec<String>,
    initiator_sends: Vec<String>,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct SuiteEnvelope {
    event_id: String,
    #[serde(rename = "type")]
    event_type: String,
    schema_version: u32,
    occurred_at: DateTime<Utc>,
    user_id: String,
    source_app: String,
    payload: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct IngestResponse {
    outcome: String,
    ledger_entry_id: Option<String>,
}
#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
struct DeliveryResponse {
    job_id: Uuid,
    link_id: Uuid,
    event_type: Option<String>,
    #[schema(value_type = SuiteDeliveryStatus)]
    status: String,
    attempt_count: u32,
    last_failure_code: Option<SuiteDeliveryFailureCode>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StartRequest {
    peer_app: String,
    return_url: String,
    #[schema(min_length = 1, max_length = 128)]
    state_nonce: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompleteRequest {
    code: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PatchRequest {
    share_xp: Option<bool>,
    reward_mode: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ReplayRequest {
    since: NaiveDate,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct AuthorizeRequest {
    client: String,
    state: String,
    code_challenge: String,
    hint: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct DenyRequest {
    client: String,
    state: String,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ExchangeRequestDto {
    #[schema(max_length = 64)]
    client: String,
    code: String,
    code_verifier: String,
    initiator_link_id: Uuid,
    link_secret: String,
    initiator_subject: String,
    initiator_suite_subject: Option<String>,
    initiator_display_name: Option<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PreviewQuery {
    #[serde(rename = "hint_domain")]
    hint_domain: Option<String>,
    client: String,
    hint: Option<String>,
}
fn api(state: &SuiteHttpState) -> Result<&dyn SuiteApi, HttpError> {
    Ok(state.api.as_ref())
}
fn user(context: SuiteUser) -> Result<SuiteUser, HttpError> {
    Ok(context)
}
fn signed(headers: &HeaderMap) -> Result<SignedHeaders, HttpError> {
    SignedHeaders::parse(
        headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.to_str().unwrap_or(""))),
    )
    .map_err(Into::into)
}

#[utoipa::path(
    summary = "List configured suite peers",
    operation_id = "suite_peers",
    description = "Requires a user bearer token. Returns configured peers; standalone mode returns an empty list.",
    get,
    path = "/suite/peers",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [PeerResponse]),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn peers(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
) -> Result<Json<Vec<PeerResponse>>, HttpError> {
    Ok(Json(
        api(&state)?
            .peers(user(context)?.id)
            .await?
            .into_iter()
            .map(|p| PeerResponse {
                id: p.id,
                display_name: p.display_name,
                sends: p.sends,
                receives: p.receives,
                reward_modes: p.reward_modes,
                scheme: p.scheme,
                web_url: p.web_url,
                share_xp_available: p.share_xp_available,
                link: p.link,
            })
            .collect(),
    ))
}

#[utoipa::path(
    summary = "Start a suite link request",
    operation_id = "suite_start",
    description = "Requires a user bearer token. Validates the return URL, supersedes an open request for this peer and creates a PKCE-bound authorization URL.",
    post,
    path = "/suite/links",
    request_body = StartRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 201, body = StartResponse),
        (status = 400, description = "suite_peer_unknown or invalid request", body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, description = "not_found: user or request not found", body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn start(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiJson(r): ApiJson<StartRequest>,
) -> Result<(StatusCode, Json<StartResponse>), HttpError> {
    let r = api(&state)?
        .start(
            user(context)?,
            LinkStartRequest {
                peer_app: r.peer_app,
                return_url: r.return_url,
                state_nonce: r.state_nonce,
            },
            Utc::now(),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(StartResponse {
            request_id: r.request_id,
            authorize_url: r.authorize_url,
        }),
    ))
}

#[utoipa::path(
    summary = "Complete an owned suite link request",
    operation_id = "suite_complete",
    description = "Requires the initiating user bearer token. Exchanges the code with the configured peer. Other owners receive 404; completed requests return the persisted link.",
    post,
    path = "/suite/links/requests/{id}/complete",
    request_body = CompleteRequest,
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 201, body = SuiteLinkResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
        (status = 502, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn complete(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(r): ApiJson<CompleteRequest>,
) -> Result<(StatusCode, Json<SuiteLinkResponse>), HttpError> {
    Ok((
        StatusCode::CREATED,
        Json(
            api(&state)?
                .complete(user(context)?, id, r.code, Utc::now())
                .await?
                .into(),
        ),
    ))
}

#[utoipa::path(
    summary = "List connected suite links",
    operation_id = "suite_list",
    description = "Requires a user bearer token. Returns the user's active and needs-attention links without credentials.",
    get,
    path = "/suite/links",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = [SuiteLinkResponse]),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn list(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
) -> Result<Json<Vec<SuiteLinkResponse>>, HttpError> {
    Ok(Json(
        api(&state)?
            .list(user(context)?.id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}

#[utoipa::path(
    summary = "Get an owned suite link",
    operation_id = "suite_get",
    description = "Requires the owning user bearer token. Returns link health and preferences; other owners receive 404.",
    get,
    path = "/suite/links/{id}",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = SuiteLinkResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn get(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<SuiteLinkResponse>, HttpError> {
    Ok(Json(api(&state)?.get(user(context)?.id, id).await?.into()))
}

#[utoipa::path(
    summary = "Re-enable suite delivery",
    operation_id = "suite_reenable",
    description = "Requires the owning user bearer token. Resets an active link's delivery circuit breaker.",
    post,
    path = "/suite/links/{id}/reenable",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = SuiteLinkResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn reenable(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<SuiteLinkResponse>, HttpError> {
    Ok(Json(
        api(&state)?
            .reenable(user(context)?.id, id, Utc::now())
            .await?
            .into(),
    ))
}

#[utoipa::path(
    summary = "Update suite link preferences",
    operation_id = "suite_update",
    description = "Requires the owning user bearer token. Updates XP sharing and the receiver-supported reward mode.",
    patch,
    path = "/suite/links/{id}",
    request_body = PatchRequest,
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = SuiteLinkResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn update(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(r): ApiJson<PatchRequest>,
) -> Result<Json<SuiteLinkResponse>, HttpError> {
    let mode = r.reward_mode.map(|m| m.parse()).transpose().map_err(|_| {
        HttpError::validation_with_code("suite_payload_invalid", "Invalid reward mode")
    })?;
    Ok(Json(
        api(&state)?
            .update(user(context)?.id, id, r.share_xp, mode, Utc::now())
            .await?
            .into(),
    ))
}

#[utoipa::path(
    summary = "Disconnect a suite link",
    operation_id = "suite_disconnect",
    description = "Requires the owning user bearer token. Revokes the local link and queues a signed revoke to its configured peer.",
    delete,
    path = "/suite/links/{id}",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn disconnect(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, HttpError> {
    api(&state)?
        .disconnect(user(context)?.id, id, Utc::now())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    summary = "Queue a suite connection test",
    operation_id = "suite_test",
    description = "Requires the owning user bearer token. Queues a signed connection test that never grants rewards.",
    post,
    path = "/suite/links/{id}/test",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 202),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 410, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn test(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, HttpError> {
    api(&state)?.test(user(context)?.id, id, Utc::now()).await?;
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    summary = "Queue suite history replay",
    operation_id = "suite_replay",
    description = "Requires the owning user bearer token. Queues history from at most 365 days ago, no more than once per link in 24 hours. Replay deliveries never grant rewards.",
    post,
    path = "/suite/links/{id}/replay",
    request_body = ReplayRequest,
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 202),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 410, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, description = "suite_replay_window: details.earliest and details.latest are ISO dates; validation_failed: invalid request schema", body = ErrorResponse),
        (status = 429, description = "suite_replay_throttled: Retry-After is the remaining 24-hour cooldown", body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn replay(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(r): ApiJson<ReplayRequest>,
) -> Result<StatusCode, HttpError> {
    api(&state)?
        .replay(user(context)?.id, id, r.since, Utc::now())
        .await?;
    Ok(StatusCode::ACCEPTED)
}

#[utoipa::path(
    summary = "List recent suite deliveries",
    operation_id = "suite_deliveries",
    description = "Requires the owning user bearer token. Returns at most twenty recent delivery jobs with safe status and failure codes.",
    get,
    path = "/suite/links/{id}/deliveries",
    security(("bearerAuth" = [])),
    params(("id" = Uuid, Path)),
    responses(
        (status = 200, body = [DeliveryResponse]),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn deliveries(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<Vec<DeliveryResponse>>, HttpError> {
    Ok(Json(
        api(&state)?
            .deliveries(user(context)?.id, id)
            .await?
            .into_iter()
            .map(|d| DeliveryResponse {
                job_id: d.job_id,
                link_id: d.link_id,
                event_type: d.event_type,
                status: d.status,
                attempt_count: d.attempt_count,
                last_failure_code: d.last_error_code.as_deref().map(Into::into),
                created_at: d.created_at,
                updated_at: d.updated_at,
            })
            .collect(),
    ))
}

#[utoipa::path(
    summary = "Preview suite authorization",
    operation_id = "suite_preview",
    description = "Requires a user bearer token. Shows negotiated types and whether the hint matches the signed-in suite identity.",
    get,
    path = "/suite/authorizations/preview",
    security(("bearerAuth" = [])),
    params(("client" = String, Query), ("hint" = Option<String>, Query), ("hint_domain" = Option<String>, Query)),
    responses(
        (status = 200, body = PreviewResponse),
        (status = 400, description = "suite_peer_unknown or invalid request", body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn preview(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    Query(q): Query<PreviewQuery>,
) -> Result<Json<PreviewResponse>, HttpError> {
    let p = api(&state)?
        .preview(user(context)?, q.client, q.hint, q.hint_domain)
        .await?;
    Ok(Json(PreviewResponse {
        peer: p.peer,
        authorizer_sends: p.authorizer_sends,
        initiator_sends: p.initiator_sends,
        existing_link: p.existing_link,
        auto_approve: p.auto_approve,
        hint_mismatch: p.hint_mismatch,
    }))
}

#[utoipa::path(
    summary = "Authorize a suite link",
    operation_id = "suite_authorize",
    description = "Requires a user bearer token. Issues a sixty-second PKCE-bound code. Matching hints auto-approve; other hints require explicit consent.",
    post,
    path = "/suite/authorizations",
    request_body = AuthorizeRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RedirectResponse),
        (status = 400, description = "suite_peer_unknown or invalid request", body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn authorize(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiJson(r): ApiJson<AuthorizeRequest>,
) -> Result<Json<RedirectResponse>, HttpError> {
    let r = api(&state)?
        .authorize(
            user(context)?,
            AuthorizationRequest {
                client: r.client,
                state: r.state,
                code_challenge: r.code_challenge,
                hint: r.hint,
            },
            Utc::now(),
        )
        .await?;
    Ok(Json(RedirectResponse {
        redirect_url: r.redirect_url,
    }))
}

#[utoipa::path(
    summary = "Deny suite authorization",
    operation_id = "suite_deny",
    description = "Requires a user bearer token. Returns an access_denied redirect to the configured initiator callback.",
    post,
    path = "/suite/authorizations/deny",
    request_body = DenyRequest,
    security(("bearerAuth" = [])),
    responses(
        (status = 200, body = RedirectResponse),
        (status = 400, description = "suite_peer_unknown or invalid request", body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn deny(
    State(state): State<SuiteHttpState>,
    Authenticated(context): Authenticated,
    ApiJson(r): ApiJson<DenyRequest>,
) -> Result<Json<RedirectResponse>, HttpError> {
    user(context)?;
    let r = api(&state)?
        .deny(AuthorizationDeny {
            client: r.client,
            state: r.state,
        })
        .await?;
    Ok(Json(RedirectResponse {
        redirect_url: r.redirect_url,
    }))
}

#[utoipa::path(
    summary = "Exchange a suite authorization code",
    operation_id = "suite_exchange",
    description = "Public PKCE-authenticated exchange, limited to twenty failed attempts per minute per configured peer. Valid codes always pass. Matching initiator link IDs return the same 201 during the code lifetime.",
    post,
    path = "/suite/links/exchange",
    request_body = ExchangeRequestDto,
    responses(
        (status = 201, body = ExchangeResponseDto),
        (status = 400, description = "suite_peer_unknown or invalid request", body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, description = "not_found: user or request not found", body = ErrorResponse),
        (status = 409, body = ErrorResponse),
        (status = 413, body = ErrorResponse),
        (status = 415, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn exchange(
    State(state): State<SuiteHttpState>,
    ApiJson(r): ApiJson<ExchangeRequestDto>,
) -> Result<(StatusCode, Json<ExchangeResponseDto>), HttpError> {
    let r = api(&state)?
        .exchange(
            ExchangeRequest {
                client: r.client,
                code: r.code,
                code_verifier: r.code_verifier,
                initiator_link_id: r.initiator_link_id,
                link_secret: r.link_secret,
                initiator_subject: r.initiator_subject,
                initiator_suite_subject: r.initiator_suite_subject,
                initiator_display_name: r.initiator_display_name,
            },
            Utc::now(),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(ExchangeResponseDto {
            link_id: r.link_id,
            subject: r.subject,
            suite_subject: r.suite_subject,
            display_name: r.display_name,
            authorizer_sends: r.authorizer_sends,
            initiator_sends: r.initiator_sends,
        }),
    ))
}

#[utoipa::path(
    summary = "Relay a suite authorization callback",
    operation_id = "suite_callback",
    description = "Public relay only. Validates state and the expected peer, echoes the client nonce and redirects to the stored return URL without exchanging the code.",
    get,
    path = "/suite/links/callback",
    params(("state" = Option<String>, Query), ("code" = Option<String>, Query), ("from" = Option<String>, Query), ("error" = Option<String>, Query)),
    responses(
        (status = 303),
        (status = 400, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn callback(
    State(state): State<SuiteHttpState>,
    Query(q): Query<LinkCallbackQuery>,
) -> Result<Response, HttpError> {
    let r = api(&state)?.callback(q, Utc::now()).await.map_err(|e| {
        if e.code() == "suite_code_invalid" {
            HttpError::bad_request_with_code("suite_code_invalid", "Invalid suite callback")
        } else {
            e.into()
        }
    })?;
    Ok((
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, r.location),
            (header::CACHE_CONTROL, "no-store".to_owned()),
            (header::REFERRER_POLICY, "no-referrer".to_owned()),
        ],
    )
        .into_response())
}

#[utoipa::path(
    summary = "Receive a signed suite revocation",
    operation_id = "suite_revoke",
    description = "Public signed control call. Verifies the raw body, source and timestamp before revoking the link. A correctly signed retry is idempotent.",
    post,
    path = "/suite/links/{remoteLinkId}/revoke",
    request_body = serde_json::Value,
    params(("remoteLinkId" = Uuid,Path),("Content-Type"=String,Header),("X-Suite-Signature"=String,Header),("X-Suite-Timestamp"=u64,Header),("X-Suite-Delivery-Id"=String,Header),("X-Suite-Source"=String,Header),("X-Suite-Replay"=Option<String>,Header)),
    responses(
        (status = 204),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 422, body = ErrorResponse),
        (status = 429, body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn revoke(
    State(state): State<SuiteHttpState>,
    ApiPath(id): ApiPath<Uuid>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<StatusCode, HttpError> {
    api(&state)?
        .revoke_from_peer(
            id,
            signed(&headers)?,
            body.map_err(|_| HttpError::from(crate::services::SuiteServiceError::PayloadInvalid))?
                .to_vec(),
            Utc::now(),
        )
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    summary = "Receive a signed suite event",
    operation_id = "suite_inbound",
    description = "Public signed event call. Unregistered sources return 401 without a database read or limiter key. Known links are limited to 120 requests per minute per link; unknown IDs share a twenty-per-minute bucket per registered source. Known-link traffic never charges the unknown-ID bucket. Rate-limited calls return 429 with Retry-After. Verifies up to 16384 raw bytes before parsing. Commits inbox and applier work together; duplicates return 200.",
    post,
    path = "/suite/inbound/{linkId}",
    request_body = SuiteEnvelope,
    params(("linkId" = Uuid,Path),("Content-Type"=String,Header),("X-Suite-Signature"=String,Header),("X-Suite-Timestamp"=u64,Header),("X-Suite-Delivery-Id"=String,Header),("X-Suite-Source"=String,Header),("X-Suite-Replay"=Option<String>,Header)),
    responses(
        (status = 200, body = IngestResponse),
        (status = 202, body = IngestResponse),
        (status = 400, body = ErrorResponse),
        (status = 401, body = ErrorResponse),
        (status = 403, body = ErrorResponse),
        (status = 404, body = ErrorResponse),
        (status = 410, body = ErrorResponse),
        (status = 422, description = "suite_payload_invalid: catalog or product applier rejected the payload; suite_event_type_unsupported: type or source mismatch", body = ErrorResponse),
        (status = 429, description = "rate_limited: link or source limit; suite_quota_exceeded: product owner quota. Retry-After gives the wait in seconds", body = ErrorResponse),
        (status = 500, body = ErrorResponse),
        (status = 504, body = ErrorResponse),
        (status = 503, body = ErrorResponse),
    ),
    tag = "suite",
)]
async fn inbound(
    State(state): State<SuiteHttpState>,
    ApiPath(id): ApiPath<Uuid>,
    headers: HeaderMap,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<IngestResponse>), HttpError> {
    let r = api(&state)?
        .ingest(
            id,
            signed(&headers)?,
            body.map_err(|_| HttpError::from(crate::services::SuiteServiceError::PayloadInvalid))?
                .to_vec(),
            Utc::now(),
        )
        .await?;
    Ok((
        if r.duplicate {
            StatusCode::OK
        } else {
            StatusCode::ACCEPTED
        },
        Json(IngestResponse {
            outcome: serde_json::to_value(r.outcome.outcome)
                .map_err(HttpError::internal)?
                .as_str()
                .ok_or_else(|| {
                    HttpError::internal(std::io::Error::other("invalid ingest outcome"))
                })?
                .to_owned(),
            ledger_entry_id: r.outcome.ledger_entry_id,
        }),
    ))
}

/// Mounts the suite routes under the product API prefix. Auth middleware must insert `SuiteUser`.
pub fn router(state: SuiteHttpState) -> axum::Router {
    use axum::routing::{get as route_get, post};
    axum::Router::new()
        .route("/suite/peers", route_get(peers))
        .route("/suite/links", route_get(list).post(start))
        .route("/suite/links/requests/{id}/complete", post(complete))
        .route("/suite/links/callback", route_get(callback))
        .route("/suite/links/exchange", post(exchange))
        .route(
            "/suite/links/{id}",
            route_get(get).patch(update).delete(disconnect),
        )
        .route("/suite/links/{id}/test", post(test))
        .route("/suite/links/{id}/reenable", post(reenable))
        .route("/suite/links/{id}/replay", post(replay))
        .route("/suite/links/{id}/deliveries", route_get(deliveries))
        .route("/suite/links/{id}/revoke", post(revoke))
        .route("/suite/authorizations/preview", route_get(preview))
        .route("/suite/authorizations", post(authorize))
        .route("/suite/authorizations/deny", post(deny))
        .route("/suite/inbound/{id}", post(inbound))
        .layer(DefaultBodyLimit::max(SUITE_MAX_BODY_BYTES))
        .with_state(state)
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    peers, start, complete, list, get, reenable, update, disconnect, test, replay, deliveries,
    preview, authorize, deny, exchange, callback, revoke, inbound
))]
pub struct SuiteOpenApi;

#[cfg(test)]
mod tests {
    use super::*;
    use utoipa::OpenApi;
    #[test]
    fn openapi_uses_the_event_envelope_wire_name_and_generic_app_ids() {
        let document = serde_json::to_value(SuiteOpenApi::openapi()).expect("OpenAPI");
        let schemas = &document["components"]["schemas"];
        let properties = &schemas["SuiteEnvelope"]["properties"];
        assert!(properties.get("sourceApp").is_some());
        assert!(properties.get("source").is_none());
        assert!(
            schemas["StartRequest"]["properties"]["peerApp"]
                .get("enum")
                .is_none()
        );
        assert!(
            document["paths"]["/suite/inbound/{linkId}"]["post"]["responses"]["422"]["description"]
                .as_str()
                .expect("payload rejection description")
                .contains("suite_payload_invalid")
        );
        assert_eq!(document["paths"].as_object().expect("paths").len(), 15);
        assert!(
            schemas["PeerResponse"]["properties"]
                .get("mappableMetrics")
                .is_none()
        );
    }
}
