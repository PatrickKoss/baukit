//! A deliberately small notes service demonstrating every Baukit integration seam.

use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use baukit_config::{HttpConfig, Validate, ValidationError, ValidationErrors};
use baukit_http::{ApiError, ApiJson, ApiPath, HttpOptions, HttpOptionsError, finalize};
use baukit_openapi::{ErrorBody, ErrorEnvelope, OpenApiMetadata};
use baukit_ops::{OpsRouter, PrometheusHandle, ReadinessError, RegistrationError, TrafficGate};
use baukit_telemetry::ServiceIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{OpenApi, ToSchema};

/// Product-owned configuration flattened beside Baukit's standard sections.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ProductConfig {
    /// Maximum number of notes held by this process.
    pub max_notes: usize,
}

impl Default for ProductConfig {
    fn default() -> Self {
        Self { max_notes: 100 }
    }
}

impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        if self.max_notes == 0 {
            Err(ValidationErrors::new(vec![ValidationError::new(
                "max_notes",
                "must be non-zero",
            )]))
        } else {
            Ok(())
        }
    }
}

/// One in-memory note returned by the API.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize, ToSchema)]
pub struct Note {
    /// Process-local note identifier.
    pub id: u64,
    /// Short note title.
    pub title: String,
    /// Free-form note body.
    pub body: String,
}

/// Request body accepted by `POST /notes`.
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct CreateNote {
    /// Short, non-empty note title.
    pub title: String,
    /// Free-form note body, limited to 1,000 characters in this example.
    pub body: String,
}

/// Shared in-memory application state.
#[derive(Clone)]
pub struct AppState {
    notes: Arc<RwLock<BTreeMap<u64, Note>>>,
    next_id: Arc<AtomicU64>,
    max_notes: usize,
}

impl AppState {
    /// Creates empty note state with a configured capacity.
    #[must_use]
    pub fn new(max_notes: usize) -> Self {
        Self {
            notes: Arc::new(RwLock::new(BTreeMap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            max_notes,
        }
    }

    /// Returns the current note count, or an error if the lock was poisoned.
    pub fn note_count(&self) -> Result<usize, io::Error> {
        self.notes
            .read()
            .map(|notes| notes.len())
            .map_err(|_| io::Error::other("notes state lock is poisoned"))
    }
}

/// Builds the public router with the complete Baukit HTTP layer stack.
pub fn api_router(state: AppState, config: &HttpConfig) -> Result<Router, HttpOptionsError> {
    let router = Router::new()
        .route("/notes", post(create_note).get(list_notes))
        .route("/notes/{id}", get(get_note))
        .route("/fail", get(fail_validation))
        .with_state(state);
    Ok(finalize(router, HttpOptions::from_config(config)?))
}

/// Builds the private operations router with state readiness and a shared traffic gate.
pub fn ops_router(
    state: AppState,
    identity: ServiceIdentity,
    metrics: PrometheusHandle,
    traffic_gate: TrafficGate,
) -> Result<Router, RegistrationError> {
    let readiness = baukit_ops::ReadinessRegistry::new();
    readiness.register_fn_default("state", move || {
        let state = state.clone();
        async move {
            state
                .note_count()
                .map(|_| ())
                .map_err(|_| ReadinessError::new("notes state is unavailable"))
        }
    })?;

    Ok(OpsRouter::new(identity, metrics)
        .with_readiness(readiness)
        .with_traffic_gate(traffic_gate)
        .into_router())
}

/// Produces the product-owned OpenAPI document with Baukit metadata conventions.
#[must_use]
pub fn openapi_document() -> utoipa::openapi::OpenApi {
    let mut document = ApiDoc::openapi();
    OpenApiMetadata::new(
        "Minimal Notes API",
        env!("CARGO_PKG_VERSION"),
        "A dependency-light reference service showing Baukit composition.",
    )
    .apply_to(&mut document);
    document
}

#[utoipa::path(
    post,
    path = "/notes",
    request_body = CreateNote,
    responses(
        (status = 201, description = "Note created", body = Note),
        (status = 400, description = "Validation failure", body = ErrorEnvelope),
        (status = 409, description = "Configured note capacity reached", body = ErrorEnvelope)
    ),
    tag = "notes"
)]
async fn create_note(
    State(state): State<AppState>,
    ApiJson(request): ApiJson<CreateNote>,
) -> Result<(StatusCode, Json<Note>), ApiError> {
    let mut details = BTreeMap::new();
    if request.title.trim().is_empty() {
        details.insert(
            "title".to_owned(),
            Value::String("must not be empty".to_owned()),
        );
    }
    if request.body.chars().count() > 1_000 {
        details.insert(
            "body".to_owned(),
            Value::String("must contain at most 1000 characters".to_owned()),
        );
    }
    if !details.is_empty() {
        return Err(ApiError::validation(details));
    }

    let mut notes = state
        .notes
        .write()
        .map_err(|_| ApiError::internal(io::Error::other("notes state lock is poisoned")))?;
    if notes.len() >= state.max_notes {
        return Err(ApiError::conflict("The note limit has been reached"));
    }

    let id = state.next_id.fetch_add(1, Ordering::Relaxed);
    let note = Note {
        id,
        title: request.title,
        body: request.body,
    };
    notes.insert(id, note.clone());
    Ok((StatusCode::CREATED, Json(note)))
}

#[utoipa::path(
    get,
    path = "/notes/{id}",
    params(("id" = u64, Path, description = "Process-local note identifier")),
    responses(
        (status = 200, description = "Note found", body = Note),
        (status = 404, description = "Note not found", body = ErrorEnvelope)
    ),
    tag = "notes"
)]
async fn get_note(
    State(state): State<AppState>,
    ApiPath(id): ApiPath<u64>,
) -> Result<Json<Note>, ApiError> {
    let notes = state
        .notes
        .read()
        .map_err(|_| ApiError::internal(io::Error::other("notes state lock is poisoned")))?;
    notes
        .get(&id)
        .cloned()
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Note not found"))
}

#[utoipa::path(
    get,
    path = "/notes",
    responses((status = 200, description = "All notes", body = [Note])),
    tag = "notes"
)]
async fn list_notes(State(state): State<AppState>) -> Result<Json<Vec<Note>>, ApiError> {
    let notes = state
        .notes
        .read()
        .map_err(|_| ApiError::internal(io::Error::other("notes state lock is poisoned")))?;
    Ok(Json(notes.values().cloned().collect()))
}

#[utoipa::path(
    get,
    path = "/fail",
    responses((status = 400, description = "Deliberate structured validation failure", body = ErrorEnvelope)),
    tag = "example"
)]
async fn fail_validation() -> Result<StatusCode, ApiError> {
    let details = BTreeMap::from([(
        "example".to_owned(),
        Value::String("this endpoint always fails".to_owned()),
    )]);
    Err(ApiError::validation(details))
}

#[derive(OpenApi)]
#[openapi(
    paths(create_note, get_note, list_notes, fail_validation),
    components(schemas(Note, CreateNote, ErrorEnvelope, ErrorBody)),
    tags(
        (name = "notes", description = "In-memory note operations"),
        (name = "example", description = "Explicit error-contract demonstration")
    )
)]
struct ApiDoc;
