use std::sync::Arc;

use axum::{
    Extension, Json, Router,
    extract::{Path, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use baukit_auth::Principal;
use baukit_http::ApiError;
use uuid::Uuid;

use super::{DataSubjectError, DataSubjectProduct, PostgresDataSubjectStore};
use crate::{
    DataSubjectRequest, OperationKind, codes, normalize_request, validate_idempotency_key,
};

#[derive(Clone)]
struct RouterState {
    store: PostgresDataSubjectStore,
    product: Arc<dyn DataSubjectProduct>,
}

/// Erasure POST and GET routes. Mount behind the product's authentication middleware.
/// Missing principals are still rejected with the standard 401 envelope.
pub fn authenticated_routes(
    store: PostgresDataSubjectStore,
    product: Arc<dyn DataSubjectProduct>,
) -> Router {
    Router::new()
        .route("/api/v1/data-subject/erasures", post(start_erasure))
        .route(
            "/api/v1/data-subject/erasures/{operationId}",
            get(erasure_status),
        )
        .with_state(RouterState { store, product })
}

/// Unauthenticated discovery with `Cache-Control: public, max-age=300`.
pub fn discovery_routes(store: PostgresDataSubjectStore) -> Router {
    Router::new()
        .route("/.well-known/baukit/data-subject", get(discovery))
        .with_state(store)
}

async fn discovery(State(store): State<PostgresDataSubjectStore>) -> impl IntoResponse {
    (
        [(CACHE_CONTROL, "public, max-age=300")],
        Json(store.discovery()),
    )
}

fn authorize(
    store: &PostgresDataSubjectStore,
    principal: Option<Extension<Principal>>,
) -> Result<String, ApiError> {
    let Extension(principal) = principal.ok_or_else(ApiError::unauthenticated)?;
    let client = principal
        .client_id()
        .filter(|id| store.config.allowed_client_ids.contains(*id));
    if principal.grants().is_some() || client.is_none() {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            codes::CLIENT_NOT_ALLOWED,
            "The machine client is not allowed",
        ));
    }
    if !principal.scopes().contains("data-subject.erase") {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            codes::INSUFFICIENT_SCOPE,
            "The required scope is missing",
        ));
    }
    Ok(client.expect("checked client identity").to_owned())
}

async fn start_erasure(
    State(state): State<RouterState>,
    principal: Option<Extension<Principal>>,
    headers: HeaderMap,
    body: Result<Json<DataSubjectRequest>, JsonRejection>,
) -> Result<Response, ApiError> {
    let client = authorize(&state.store, principal)?;
    let values = headers.get_all("Idempotency-Key");
    let key = values.iter().next().and_then(|header| header.to_str().ok());
    if values.iter().count() != 1 || validate_idempotency_key(key).is_err() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            codes::IDEMPOTENCY_KEY_INVALID,
            "A valid Idempotency-Key is required",
        ));
    }
    let Json(request) = body.map_err(|_| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            codes::REQUEST_INVALID,
            "The data subject request is invalid",
        )
    })?;
    let request = normalize_request(
        &request,
        &state.store.config.limits,
        &state.store.config.identifier_types,
    )
    .map_err(|error| {
        ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            error.code(),
            "The data subject request is invalid",
        )
    })?;
    let operation = state
        .store
        .start_erasure(
            &client,
            key.expect("validated key"),
            &request,
            state.product.as_ref(),
        )
        .await
        .map_err(api_error)?;
    let status = if operation.status.is_terminal() {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((status, Json(operation)).into_response())
}

async fn erasure_status(
    State(state): State<RouterState>,
    principal: Option<Extension<Principal>>,
    Path(id): Path<String>,
) -> Result<Response, ApiError> {
    let client = authorize(&state.store, principal)?;
    let id = Uuid::parse_str(&id).map_err(|_| not_found())?;
    let operation = state
        .store
        .status(&client, OperationKind::Erasure, id)
        .await
        .map_err(api_error)?
        .ok_or_else(not_found)?;
    Ok(Json(operation).into_response())
}

fn not_found() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        codes::OPERATION_NOT_FOUND,
        "The data subject operation was not found",
    )
}

fn api_error(error: DataSubjectError) -> ApiError {
    if error == DataSubjectError::Conflict {
        ApiError::new(
            StatusCode::CONFLICT,
            codes::IDEMPOTENCY_CONFLICT,
            "The key was used with a different request",
        )
    } else {
        // Product database errors can contain lookup values. Never retain their causes.
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "An internal error occurred",
        )
    }
}
