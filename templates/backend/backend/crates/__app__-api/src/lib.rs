use std::collections::BTreeMap;

{% if context.auth_oidc %}use axum::extract::FromRef;
{% endif %}use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
{% if context.auth_oidc %}use baukit_auth::{AuthState, Principal};
{% endif %}use baukit_config::HttpConfig;
use baukit_http::{
    ApiError, ApiJson, ApiPath, ErrorBody, ErrorEnvelope, HttpOptions, HttpOptionsError,
};
use baukit_openapi::OpenApiMetadata;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};
use uuid::Uuid;

use {{ context.app_crate }}_domain::Item;
use {{ context.app_crate }}_ports::RepositoryError;
use {{ context.app_crate }}_services::{ItemService, ServiceError{% if context.auth_oidc %}, UserService{% endif %}};

#[derive(Clone)]
pub struct ApiState {
    pub items: ItemService,
{% if context.auth_oidc %}    pub users: UserService,
    pub auth: AuthState,
{% endif %}}

{% if context.auth_oidc %}impl FromRef<ApiState> for AuthState {
    fn from_ref(state: &ApiState) -> Self {
        state.auth.clone()
    }
}

{% endif %}pub fn router(state: ApiState, config: &HttpConfig) -> Result<Router, HttpOptionsError> {
    let router = Router::new()
        .route("/items", get(list_items).post(create_item))
        .route(
            "/items/{id}",
            get(get_item).put(update_item).delete(delete_item),
        )
{% if context.auth_oidc %}        .route("/me", get(current_user))
{% endif %}        .with_state(state);
    Ok(baukit_http::finalize(
        router,
        HttpOptions::from_config(config)?,
    ))
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
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
pub struct SaveItemRequest {
    pub name: String,
}

{% if context.auth_oidc %}#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
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
{% if context.auth_oidc %}    security(("bearerAuth" = [])),
{% endif %}    responses(
        (status = 200, description = "Items", body = [ItemDto]){% if context.auth_oidc %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
{% if context.auth_oidc %}async fn list_items(
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
{% if context.auth_oidc %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 200, description = "Item", body = ItemDto),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_oidc %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn get_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_oidc %}    _principal: Principal,
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
{% if context.auth_oidc %}    security(("bearerAuth" = [])),
{% endif %}    request_body = SaveItemRequest,
    responses(
        (status = 201, description = "Created", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 409, description = "Conflict", body = ErrorEnvelope){% if context.auth_oidc %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn create_item(
    State(state): State<ApiState>,
{% if context.auth_oidc %}    _principal: Principal,
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
{% if context.auth_oidc %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    request_body = SaveItemRequest,
    responses(
        (status = 200, description = "Updated", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_oidc %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn update_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_oidc %}    _principal: Principal,
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
{% if context.auth_oidc %}    security(("bearerAuth" = [])),
{% endif %}    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = ErrorEnvelope){% if context.auth_oidc %},
        (status = 401, description = "Authentication required", body = ErrorEnvelope){% endif %}
    ),
    tag = "items"
)]
async fn delete_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
{% if context.auth_oidc %}    _principal: Principal,
{% endif %}) -> Result<StatusCode, ApiError> {
    state.items.delete(id).await.map_err(map_service_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn map_service_error(error: ServiceError) -> ApiError {
    match error {
        ServiceError::NotFound => ApiError::not_found("Item not found"),
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
        "Generated Baukit item service.",
    );
{% if context.auth_oidc %}    let metadata = metadata.bearer_auth();
{% endif %}    metadata.apply_to(&mut document);
    document
}

#[derive(OpenApi)]
#[openapi(
    paths(list_items, get_item, create_item, update_item, delete_item{% if context.auth_oidc %}, current_user{% endif %}),
    components(schemas(ItemDto, SaveItemRequest, ErrorEnvelope, ErrorBody{% if context.auth_oidc %}, CurrentUserDto{% endif %})),
    tags(
        (name = "items", description = "Example item operations"){% if context.auth_oidc %},
        (name = "auth", description = "Protected identity example"){% endif %}
    )
)]
struct ApiDoc;
