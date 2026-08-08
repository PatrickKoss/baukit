use std::collections::BTreeMap;

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use baukit_config::HttpConfig;
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
use {{ context.app_crate }}_services::{ItemService, ServiceError};

#[derive(Clone)]
pub struct ApiState {
    pub items: ItemService,
}

pub fn router(state: ApiState, config: &HttpConfig) -> Result<Router, HttpOptionsError> {
    let router = Router::new()
        .route("/items", get(list_items).post(create_item))
        .route(
            "/items/{id}",
            get(get_item).put(update_item).delete(delete_item),
        )
        .with_state(state);
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

#[utoipa::path(
    get,
    path = "/items",
    responses((status = 200, description = "Items", body = [ItemDto])),
    tag = "items"
)]
async fn list_items(State(state): State<ApiState>) -> Result<Json<Vec<ItemDto>>, ApiError> {
    let items = state
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
    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 200, description = "Item", body = ItemDto),
        (status = 404, description = "Not found", body = ErrorEnvelope)
    ),
    tag = "items"
)]
async fn get_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<Json<ItemDto>, ApiError> {
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
    request_body = SaveItemRequest,
    responses(
        (status = 201, description = "Created", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 409, description = "Conflict", body = ErrorEnvelope)
    ),
    tag = "items"
)]
async fn create_item(
    State(state): State<ApiState>,
    ApiJson(request): ApiJson<SaveItemRequest>,
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
    params(("id" = Uuid, Path, description = "Item identifier")),
    request_body = SaveItemRequest,
    responses(
        (status = 200, description = "Updated", body = ItemDto),
        (status = 400, description = "Invalid request", body = ErrorEnvelope),
        (status = 404, description = "Not found", body = ErrorEnvelope)
    ),
    tag = "items"
)]
async fn update_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
    ApiJson(request): ApiJson<SaveItemRequest>,
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
    params(("id" = Uuid, Path, description = "Item identifier")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = ErrorEnvelope)
    ),
    tag = "items"
)]
async fn delete_item(
    State(state): State<ApiState>,
    ApiPath(id): ApiPath<Uuid>,
) -> Result<StatusCode, ApiError> {
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
    OpenApiMetadata::new(
        "{{ context.app_name }} API",
        env!("CARGO_PKG_VERSION"),
        "Generated Baukit item service.",
    )
    .apply_to(&mut document);
    document
}

#[derive(OpenApi)]
#[openapi(
    paths(list_items, get_item, create_item, update_item, delete_item),
    components(schemas(ItemDto, SaveItemRequest, ErrorEnvelope, ErrorBody)),
    tags((name = "items", description = "Example item operations"))
)]
struct ApiDoc;
