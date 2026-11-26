//! Operator hook that requeues failed jobs, for the private operations listener only.
//!
//! [`requeue_failed_jobs_router`] serves `POST /admin/jobs/requeue` with a JSON
//! body `{"jobType": "<type>", "ids": ["<uuid>", ...]}` and answers
//! `{"requeued": [...]}` with the ids that went back to pending. The route has
//! no authentication: merge it into the [`OpsRouter`](crate::OpsRouter) output
//! that binds the private operations port, never into the public router.
//!
//! ```no_run
//! # fn compose(ops: axum::Router, store: baukit_jobs::PostgresJobStore) -> axum::Router {
//! use std::sync::Arc;
//! use baukit_ops::jobs_admin::requeue_failed_jobs_router;
//!
//! const JOB_TYPES: &[&str] = &["email.send"];
//! ops.merge(requeue_failed_jobs_router(
//!     Arc::new(store),
//!     JOB_TYPES,
//! ))
//! # }
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use baukit_jobs::{PostgresJobStore, StoreError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Path of the requeue route.
pub const REQUEUE_PATH: &str = "/admin/jobs/requeue";

/// A boxed future returned by [`FailedJobRequeue`].
pub type RequeueFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Vec<Uuid>, StoreError>> + Send + 'a>>;

/// Port that returns failed jobs to pending.
pub trait FailedJobRequeue: Send + Sync + 'static {
    /// Requeues the failed jobs of `job_type` among `ids` and returns those requeued.
    fn requeue_failed<'a>(&'a self, job_type: &'a str, ids: &'a [Uuid]) -> RequeueFuture<'a>;
}

impl FailedJobRequeue for PostgresJobStore {
    fn requeue_failed<'a>(&'a self, job_type: &'a str, ids: &'a [Uuid]) -> RequeueFuture<'a> {
        Box::pin(Self::requeue_failed(self, job_type, ids))
    }
}

#[derive(Clone)]
struct RequeueState {
    requeue: Arc<dyn FailedJobRequeue>,
    job_types: &'static [&'static str],
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RequeueRequest {
    job_type: String,
    ids: Vec<Uuid>,
}

#[derive(Serialize)]
struct RequeueResponse {
    requeued: Vec<Uuid>,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
}

/// Builds the requeue route. Requests for a type outside `job_types` get `400`.
pub fn requeue_failed_jobs_router(
    requeue: Arc<dyn FailedJobRequeue>,
    job_types: &'static [&'static str],
) -> Router {
    Router::new()
        .route(REQUEUE_PATH, post(requeue_failed))
        .with_state(RequeueState { requeue, job_types })
}

async fn requeue_failed(
    State(state): State<RequeueState>,
    request: Result<Json<RequeueRequest>, JsonRejection>,
) -> Response {
    let Ok(Json(request)) = request else {
        return error(
            StatusCode::BAD_REQUEST,
            "body must be {\"jobType\", \"ids\"}",
        );
    };
    if !state.job_types.contains(&request.job_type.as_str()) {
        return error(StatusCode::BAD_REQUEST, "unknown job type");
    }
    match state
        .requeue
        .requeue_failed(&request.job_type, &request.ids)
        .await
    {
        Ok(requeued) => (StatusCode::OK, Json(RequeueResponse { requeued })).into_response(),
        Err(StoreError::InvalidInput(message)) => error(StatusCode::BAD_REQUEST, &message),
        Err(store_error) => {
            tracing_error(&store_error);
            error(StatusCode::SERVICE_UNAVAILABLE, "job store unavailable")
        }
    }
}

fn tracing_error(store_error: &StoreError) {
    baukit_telemetry::tracing::error!(error = %store_error, "failed-job requeue failed");
}

fn error(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(ErrorResponse {
            error: message.to_owned(),
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt as _;

    use super::*;

    const TYPES: &[&str] = &["email.send"];
    const BODY_LIMIT: usize = 4096;

    #[derive(Default)]
    struct FakeRequeue {
        calls: Mutex<Vec<(String, Vec<Uuid>)>>,
    }

    impl FailedJobRequeue for FakeRequeue {
        fn requeue_failed<'a>(&'a self, job_type: &'a str, ids: &'a [Uuid]) -> RequeueFuture<'a> {
            Box::pin(async move {
                self.calls
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push((job_type.to_owned(), ids.to_vec()));
                if ids.is_empty() {
                    return Err(StoreError::InvalidInput("ids must not be empty".to_owned()));
                }
                Ok(ids.to_vec())
            })
        }
    }

    async fn post_json(fake: Arc<FakeRequeue>, body: String) -> (StatusCode, serde_json::Value) {
        let response = requeue_failed_jobs_router(fake, TYPES)
            .oneshot(
                Request::post(REQUEUE_PATH)
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), BODY_LIMIT)
            .await
            .expect("body");
        (status, serde_json::from_slice(&bytes).expect("json"))
    }

    #[tokio::test]
    async fn requeues_ids_of_a_declared_type() {
        let fake = Arc::new(FakeRequeue::default());
        let id = Uuid::now_v7();
        let (status, body) = post_json(
            fake.clone(),
            format!(r#"{{"jobType":"email.send","ids":["{id}"]}}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, serde_json::json!({"requeued": [id.to_string()]}));
        assert_eq!(fake.calls.lock().expect("calls").len(), 1);
    }

    #[tokio::test]
    async fn rejects_undeclared_types_and_bad_bodies_without_calling_the_store() {
        let fake = Arc::new(FakeRequeue::default());
        let (status, _) =
            post_json(fake.clone(), r#"{"jobType":"other","ids":[]}"#.to_owned()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = post_json(fake.clone(), r#"{"jobType":"email.send"}"#.to_owned()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(fake.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn store_input_errors_are_bad_requests() {
        let fake = Arc::new(FakeRequeue::default());
        let (status, body) =
            post_json(fake, r#"{"jobType":"email.send","ids":[]}"#.to_owned()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "ids must not be empty");
    }
}
