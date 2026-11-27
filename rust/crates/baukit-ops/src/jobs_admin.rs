//! Operator hook that requeues failed jobs, for the private operations listener only.
//!
//! [`requeue_failed_jobs_router`] serves `POST /admin/jobs/requeue` with a JSON
//! body `{"jobType": "<type>", "ids": ["<uuid>", ...]}` and answers
//! `{"requeued": [...]}` with the ids that went back to pending. Every request
//! needs `Authorization: Bearer <token>` matching the admin token; a missing or
//! wrong token gets `401` in the standard error envelope. Merge the router into
//! the [`OpsRouter`](crate::OpsRouter) output that binds the private operations
//! port, never into the public router.
//!
//! ```no_run
//! # fn compose(ops: axum::Router, store: baukit_jobs::PostgresJobStore, token: baukit_config::Secret<String>) -> axum::Router {
//! use std::sync::Arc;
//! use baukit_ops::jobs_admin::requeue_failed_jobs_router;
//!
//! const JOB_TYPES: &[&str] = &["email.send"];
//! ops.merge(requeue_failed_jobs_router(
//!     Arc::new(store),
//!     JOB_TYPES,
//!     token,
//! ))
//! # }
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use axum::{
    Json, Router,
    extract::{State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION},
    response::{IntoResponse, Response},
    routing::post,
};
use baukit_config::Secret;
use baukit_http::ApiError;
use baukit_jobs::{PostgresJobStore, StoreError};
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq as _;
use uuid::Uuid;

/// Path of the requeue route.
pub const REQUEUE_PATH: &str = "/admin/jobs/requeue";

const BEARER_PREFIX: &str = "Bearer ";
const STORE_RETRY_AFTER: std::time::Duration = std::time::Duration::from_secs(5);

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
    admin_token: Arc<Secret<String>>,
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

/// Builds the requeue route guarded by `admin_token`. Requests for a type
/// outside `job_types` get `400`.
pub fn requeue_failed_jobs_router(
    requeue: Arc<dyn FailedJobRequeue>,
    job_types: &'static [&'static str],
    admin_token: Secret<String>,
) -> Router {
    Router::new()
        .route(REQUEUE_PATH, post(requeue_failed))
        .with_state(RequeueState {
            requeue,
            job_types,
            admin_token: Arc::new(admin_token),
        })
}

fn bearer_matches(headers: &HeaderMap, expected: &Secret<String>) -> bool {
    let Some(presented) = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix(BEARER_PREFIX))
    else {
        return false;
    };
    let expected = expected.expose().as_bytes();
    !expected.is_empty() && bool::from(presented.as_bytes().ct_eq(expected))
}

async fn requeue_failed(
    State(state): State<RequeueState>,
    headers: HeaderMap,
    request: Result<Json<RequeueRequest>, JsonRejection>,
) -> Response {
    if !bearer_matches(&headers, &state.admin_token) {
        return ApiError::unauthenticated().into_response();
    }
    let Ok(Json(request)) = request else {
        return ApiError::bad_request("body must be {\"jobType\", \"ids\"}").into_response();
    };
    if !state.job_types.contains(&request.job_type.as_str()) {
        return ApiError::bad_request("unknown job type").into_response();
    }
    match state
        .requeue
        .requeue_failed(&request.job_type, &request.ids)
        .await
    {
        Ok(requeued) => (StatusCode::OK, Json(RequeueResponse { requeued })).into_response(),
        Err(StoreError::InvalidInput(message)) => ApiError::bad_request(message).into_response(),
        Err(store_error) => {
            baukit_telemetry::tracing::error!(error = %store_error, "failed-job requeue failed");
            ApiError::unavailable(STORE_RETRY_AFTER).into_response()
        }
    }
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

    const TOKEN: &str = "ops-admin-token";

    async fn post_with(
        fake: Arc<FakeRequeue>,
        authorization: Option<&str>,
        body: String,
    ) -> (StatusCode, serde_json::Value) {
        let router = requeue_failed_jobs_router(fake, TYPES, Secret::new(TOKEN.to_owned()));
        let mut request = Request::post(REQUEUE_PATH).header("content-type", "application/json");
        if let Some(authorization) = authorization {
            request = request.header("authorization", authorization);
        }
        let response = router
            .oneshot(request.body(Body::from(body)).expect("request"))
            .await
            .expect("response");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), BODY_LIMIT)
            .await
            .expect("body");
        (status, serde_json::from_slice(&bytes).expect("json"))
    }

    async fn post_json(fake: Arc<FakeRequeue>, body: String) -> (StatusCode, serde_json::Value) {
        post_with(fake, Some(&format!("Bearer {TOKEN}")), body).await
    }

    #[tokio::test]
    async fn missing_or_wrong_tokens_get_401_without_calling_the_store() {
        let fake = Arc::new(FakeRequeue::default());
        let body = format!(r#"{{"jobType":"email.send","ids":["{}"]}}"#, Uuid::now_v7());
        for authorization in [
            None,
            Some("Bearer wrong-token"),
            Some("Bearer ops-admin-tokenx"),
            Some(TOKEN),
            Some("Basic b3BzOmFkbWlu"),
        ] {
            let (status, response) = post_with(fake.clone(), authorization, body.clone()).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{authorization:?}");
            assert_eq!(response["error"]["code"], "unauthenticated");
            assert!(!response.to_string().contains(TOKEN));
        }
        assert!(fake.calls.lock().expect("calls").is_empty());
    }

    #[tokio::test]
    async fn an_empty_admin_token_rejects_every_request() {
        let router = requeue_failed_jobs_router(
            Arc::new(FakeRequeue::default()),
            TYPES,
            Secret::new(String::new()),
        );
        let response = router
            .oneshot(
                Request::post(REQUEUE_PATH)
                    .header("content-type", "application/json")
                    .header("authorization", "Bearer ")
                    .body(Body::from(r#"{"jobType":"email.send","ids":[]}"#))
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
        assert_eq!(body["error"]["message"], "ids must not be empty");
    }
}
