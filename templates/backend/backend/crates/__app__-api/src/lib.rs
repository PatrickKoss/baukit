{% if context.auth_enabled %}use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Json, Router,
    extract::{FromRef, Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get},
};
use baukit_auth::{AuthState, Principal};
{% else %}use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
{% endif %}use baukit_config::HttpConfig;
{% if context.auth_enabled %}use baukit_erasure::{
    ErasureError, ErasureOutcome, ErasureService, ProductErasure, reject_fenced_subject,
};
{% endif %}
use baukit_http::{
    ApiError, ApiJson, ApiPath, ErrorBody, ErrorEnvelope, HttpOptions, HttpOptionsError,
    JsonRejectionCodes,
};
use baukit_openapi::{ErrorResponseRules, OpenApiMetadata, OperationCondition as When};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use {{ context.app_crate }}_domain::Item;
use {{ context.app_crate }}_ports::RepositoryError;
use {{ context.app_crate }}_services::ItemService;
use {{ context.app_crate }}_services::ServiceError;
{% if context.auth_enabled %}use {{ context.app_crate }}_services::UserService;
{% endif %}
#[derive(Clone)]
pub struct ApiState {
    pub items: ItemService,
{% if context.auth_enabled %}    pub users: UserService,
    pub auth: AuthState,
    pub erasure: ErasureApi,
{% endif %}}

{% if context.auth_enabled %}#[derive(Clone)]
pub struct ErasureApi {
    pub service: ErasureService,
    pub product: Arc<dyn ProductErasure>,
}

impl FromRef<ApiState> for AuthState {
    fn from_ref(state: &ApiState) -> Self {
        state.auth.clone()
    }
}

{% endif %}pub fn router(state: ApiState, config: &HttpConfig) -> Result<Router, HttpOptionsError> {
    finalize_api(routes(state), config)
}

pub fn routes(state: ApiState) -> Router {
    let protected = Router::new()
        .route("/items", get(list_items).post(create_item))
        .route(
            "/items/{id}",
            get(get_item).put(update_item).delete(delete_item),
        );
{% if context.auth_enabled %}    let protected = protected
        .route("/me", get(current_user))
        .route_layer(middleware::from_fn_with_state(state.clone(), erasure_fence));
    let reconciliation = Router::new()
        .route("/me", delete(erase_current_user))
        .route("/me/erasures/{operationId}", get(erasure_status));
    protected.merge(reconciliation).with_state(state)
{% else %}    protected.with_state(state)
{% endif %}}

/// Applies the Baukit HTTP layers last, so CORS, request IDs, and the cache
/// policy also cover responses from authentication and rate-limit layers.
pub fn finalize_api(router: Router, config: &HttpConfig) -> Result<Router, HttpOptionsError> {
    Ok(baukit_http::finalize(
        router,
        HttpOptions::from_config(config)?{% if context.mcp %}
            .with_additional_allowed_headers(baukit_mcp::ALLOWED_HEADERS)?
            .with_additional_exposed_headers(["www-authenticate"])?
            {% endif %}.with_json_rejection_codes(JsonRejectionCodes::default()),
    ))
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ItemDto {
    pub id: Uuid,
    pub name: String,
}

impl From<Item> for ItemDto {
    fn from(item: Item) -> Self {
        Self {
            id: item.id,
            name: item.name,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SaveItemRequest {
    pub name: String,
}

{% if context.auth_enabled %}#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErasureStatusDto {
    Pending,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ErasureDto {
    pub status: ErasureStatusDto,
    pub operation_id: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

impl From<ErasureOutcome> for ErasureDto {
    fn from(outcome: ErasureOutcome) -> Self {
        Self {
            status: match outcome.status {
                baukit_erasure::ErasureState::Pending => ErasureStatusDto::Pending,
                baukit_erasure::ErasureState::Completed => ErasureStatusDto::Completed,
                baukit_erasure::ErasureState::Failed => ErasureStatusDto::Failed,
            },
            operation_id: outcome.operation_id,
            completed_at: outcome.completed_at.map(|time| time.to_rfc3339()),
        }
    }
}

pub async fn erasure_fence(
    State(state): State<ApiState>,
    principal: Principal,
    request: Request,
    next: Next,
) -> Result<Response, ErasureError> {
    reject_fenced_subject(state.erasure.service.store(), principal.subject()).await?;
    Ok(next.run(request).await)
}

#[utoipa::path(
    delete, path = "/me", security(("bearerAuth" = [])), tag = "auth",
    params(("Idempotency-Key" = String, Header, description = "Required 16 to 128 visible ASCII characters")),
    responses(
        (status = 200, description = "Identity and product data deleted", body = ErasureDto),
        (status = 202, description = "Product data deleted; identity deletion queued", body = ErasureDto, headers(("Location" = String, description = "Operation status URL"))),
        (status = 400, description = "erasure_idempotency_key_invalid", body = ErrorEnvelope),
        (status = 401, description = "unauthenticated or profile_erased", body = ErrorEnvelope),
        (status = 409, description = "erasure_idempotency_conflict", body = ErrorEnvelope)
    )
)]
async fn erase_current_user(
    State(state): State<ApiState>,
    principal: Principal,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let key = baukit_http::IdempotencyKeyRule::new(16, 128)
        .required(&headers)
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "erasure_idempotency_key_invalid",
                "A valid Idempotency-Key is required",
            )
        })?;
    let erasure = &state.erasure;
    let result = erasure
        .service
        .erase(principal.subject(), key.as_str(), erasure.product.as_ref())
        .await;
    let outcome = match result {
        Ok(outcome) => outcome,
        Err(error) => return Ok(error.into_response()),
    };
    let status = outcome.status_code();
    let location = format!("/me/erasures/{}", outcome.operation_id);
    let mut response = (status, Json(ErasureDto::from(outcome))).into_response();
    if status == StatusCode::ACCEPTED {
        response.headers_mut().insert(
            header::LOCATION,
            location.parse().map_err(ApiError::internal)?,
        );
    }
    Ok(response)
}

#[utoipa::path(
    get, path = "/me/erasures/{operationId}", security(("bearerAuth" = [])), tag = "auth",
    params(("operationId" = Uuid, Path)),
    responses(
        (status = 200, description = "Current erasure state", body = ErasureDto),
        (status = 401, description = "unauthenticated", body = ErrorEnvelope),
        (status = 404, description = "erasure_operation_not_found", body = ErrorEnvelope)
    )
)]
async fn erasure_status(
    State(state): State<ApiState>,
    principal: Principal,
    ApiPath(operation): ApiPath<Uuid>,
) -> Result<Response, ApiError> {
    let erasure = &state.erasure;
    match erasure
        .service
        .store()
        .status(principal.subject(), operation)
        .await
    {
        Ok(Some(outcome)) => Ok(Json(ErasureDto::from(outcome)).into_response()),
        Ok(None) => Err(ApiError::new(
            StatusCode::NOT_FOUND,
            "erasure_operation_not_found",
            "Erasure operation not found",
        )),
        Err(error) => Ok(error.into_response()),
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CurrentUserDto {
    pub id: Uuid,
    pub subject: String,
}

#[utoipa::path(
    get,
    path = "/me",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "Current internal user", body = CurrentUserDto),
        (status = 401, description = "Authentication required", body = ErrorEnvelope)
    ),
    tag = "auth"
)]
async fn current_user(
    State(state): State<ApiState>,
    principal: Principal,
) -> Result<Json<CurrentUserDto>, ApiError> {
    let user = state
        .users
        .resolve_subject(principal.subject())
        .await
        .map_err(map_service_error)?;
    Ok(Json(CurrentUserDto {
        id: user.id,
        subject: user.subject,
    }))
}

{% endif %}#[utoipa::path(
    get,
    path = "/items",
{% if context.auth_enabled %}    security(("bearerAuth" = [])),
{% endif %}    responses(
        (status = 200, description = "Items", body = [ItemDto]){% if context.auth_enabled %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
{% if context.auth_enabled %}async fn list_items(
    State(state): State<ApiState>,
    _principal: Principal,
) -> Result<Json<Vec<ItemDto>>, ApiError> {
{% else %}async fn list_items(State(state): State<ApiState>) -> Result<Json<Vec<ItemDto>>, ApiError> {
{% endif %}    let items = state
        .items
        .list()
        .await
        .map_err(map_service_error)?
        .into_iter()
        .map(Into::into)
        .collect();
    Ok(Json(items))
}

#[utoipa::path(
    get,
    path = "/items/{id}",
{% if context.auth_enabled %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 200, description = "Item", body = ItemDto),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_enabled %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn get_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_enabled %}    _principal: Principal,
{% endif %}) -> Result<Json<ItemDto>, ApiError> {
    state
        .items
        .get(id)
        .await
        .map(ItemDto::from)
        .map(Json)
        .map_err(map_service_error)
}

#[utoipa::path(
    post,
    path = "/items",
{% if context.auth_enabled %}    security(("bearerAuth" = [])),
{% endif %}    request_body = SaveItemRequest,
    responses(
        (status = 201, description = "Created", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 409, description = "Conflict", body = ErrorEnvelope){% if context.auth_enabled %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn create_item(
    State(state): State<ApiState>,
{% if context.auth_enabled %}    _principal: Principal,
{% endif %}    ApiJson(request): ApiJson<SaveItemRequest>,
) -> Result<(StatusCode, Json<ItemDto>), ApiError> {
    let item = state
        .items
        .create(request.name)
        .await
        .map_err(map_service_error)?;
    Ok((StatusCode::CREATED, Json(item.into())))
}

#[utoipa::path(
    put,
    path = "/items/{id}",
{% if context.auth_enabled %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    request_body = SaveItemRequest,
    responses(
        (status = 200, description = "Updated", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_enabled %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn update_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_enabled %}    _principal: Principal,
{% endif %}    ApiJson(request): ApiJson<SaveItemRequest>,
) -> Result<Json<ItemDto>, ApiError> {
    state
        .items
        .update(id, request.name)
        .await
        .map(ItemDto::from)
        .map(Json)
        .map_err(map_service_error)
}

#[utoipa::path(
    delete,
    path = "/items/{id}",
{% if context.auth_enabled %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_enabled %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn delete_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_enabled %}    _principal: Principal,
{% endif %}) -> Result<StatusCode, ApiError> {
    state.items.delete(id).await.map_err(map_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn map_service_error(error: ServiceError) -> ApiError {
    match error {
{% if context.auth_enabled %}        ServiceError::Repository(RepositoryError::ProfileErased) => ApiError::new(
            StatusCode::UNAUTHORIZED,
            "profile_erased",
            "The profile has been erased",
        ),
{% endif %}        ServiceError::NotFound => ApiError::not_found("Item not found"),
        ServiceError::Invalid(error) => {
            let details = BTreeMap::from([("name".to_owned(), Value::String(error.to_string()))]);
            ApiError::validation(details)
        }
        ServiceError::Repository(RepositoryError::Conflict) => {
            ApiError::conflict("Item already exists")
        }
        error @ ServiceError::Repository(RepositoryError::Unavailable(_)) => {
            ApiError::internal(error)
        }
    }
}

#[must_use]
pub fn openapi_document() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    let metadata = OpenApiMetadata::new(
        "{{ context.app_name }} API",
        env!("CARGO_PKG_VERSION"),
{% if context.auth_enabled %}        "Generated Baukit item service. Erasure errors: erasure_idempotency_key_invalid, erasure_idempotency_conflict, erasure_operation_not_found, profile_erased.",
{% else %}        "Generated Baukit item service.",
{% endif %}    );
{% if context.auth_enabled %}    let metadata = metadata.bearer_auth();
{% endif %}    metadata.apply_to(&mut document);
    error_response_rules().apply(&mut document);
    document
}

fn error_response_rules() -> ErrorResponseRules {
    ErrorResponseRules::new()
        .status(When::HasPathParameter, 400, "The path is invalid.")
        .status(
            When::HasRequestBody,
            400,
            "The request body is not valid JSON.",
        )
        .status(When::HasRequestBody, 413, "The request body is too large.")
        .status(
            When::HasRequestBody,
            415,
            "The request content type is not JSON.",
        )
        .status(
            When::HasRequestBody,
            422,
            "The request body does not match the schema.",
        )
        .status(When::HasPathParameter, 404, "The resource was not found.")
{% if context.auth_enabled %}        .status(
            When::Secured,
            401,
            "A valid bearer credential is required; profile_erased rejects fenced subjects.",
        )
        .status(
            When::Always,
            429,
            "The rate limit was exceeded; wait for Retry-After.",
        )
{% endif %}        .status(When::Always, 500, "An internal error occurred.")
        .status(
            When::Always,
            504,
            "The request deadline passed; a write may have committed.",
        )
        .standard_headers()
}

#[derive(OpenApi)]
#[openapi(
    paths(list_items, get_item, create_item, update_item, delete_item{% if context.auth_enabled %}, current_user, erase_current_user, erasure_status{% endif %}),
    components(schemas(ItemDto, SaveItemRequest, ErrorEnvelope, ErrorBody{% if context.auth_enabled %}, CurrentUserDto, ErasureDto{% endif %})),
    tags(
        (name = "items", description = "Example item operations"){% if context.auth_enabled %},
        (name = "auth", description = "Protected identity example"){% endif %}
    )
)]
struct ApiDoc;
