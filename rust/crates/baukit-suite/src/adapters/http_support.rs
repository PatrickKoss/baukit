use crate::{
    domain::LinkProtocolError,
    ports::SuiteStoreError,
    services::{SuiteApi, SuiteServiceError, SuiteUser},
};
use axum::{
    extract::FromRequestParts,
    http::{StatusCode, request::Parts},
    response::{IntoResponse, Response},
};
use baukit_http::ApiError;
use std::sync::Arc;

#[derive(Clone)]
pub struct SuiteHttpState {
    pub api: Arc<dyn SuiteApi>,
}

pub(super) struct Authenticated(pub SuiteUser);
impl<S: Send + Sync> FromRequestParts<S> for Authenticated {
    type Rejection = HttpError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<SuiteUser>()
            .cloned()
            .map(Self)
            .ok_or_else(|| ApiError::unauthenticated().into())
    }
}
pub(super) use baukit_http::ApiJson;

pub(super) struct HttpError(ApiError);
impl IntoResponse for HttpError {
    fn into_response(self) -> Response {
        self.0.into_response()
    }
}
impl From<ApiError> for HttpError {
    fn from(error: ApiError) -> Self {
        Self(error)
    }
}
impl HttpError {
    pub(super) fn validation_with_code(code: &str, message: &str) -> Self {
        ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, code, message).into()
    }
    pub(super) fn bad_request_with_code(code: &str, message: &str) -> Self {
        ApiError::new(StatusCode::BAD_REQUEST, code, message).into()
    }
    fn internal_response(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        tracing::error!(%error, "Suite request failed");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_server_error",
            "Internal server error",
        )
        .into()
    }
    pub(super) fn internal(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::internal_response(error)
    }
}
impl From<SuiteServiceError> for HttpError {
    fn from(error: SuiteServiceError) -> Self {
        use LinkProtocolError as P;
        use SuiteServiceError as E;
        use SuiteStoreError as S;
        let code = error.code();
        let status = match &error {
            E::Protocol(P::NotFound) | E::Store(S::NotFound) => StatusCode::NOT_FOUND,
            E::Protocol(P::ReturnUrlInvalid | P::PeerUnknown) => StatusCode::BAD_REQUEST,
            E::Protocol(P::Disabled) => StatusCode::FORBIDDEN,
            E::Protocol(P::AccountMismatch) | E::LinkInactive | E::Store(S::LinkInactive) => {
                StatusCode::CONFLICT
            }
            E::SignatureInvalid => StatusCode::UNAUTHORIZED,
            E::LinkRevoked | E::Store(S::LinkRevoked) => StatusCode::GONE,
            E::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            E::PeerUnreachable => StatusCode::BAD_GATEWAY,
            E::RateLimited(_) | E::Store(S::ReplayTooSoon(_)) => StatusCode::TOO_MANY_REQUESTS,
            E::ReplayWindow { .. }
            | E::Protocol(P::CodeInvalid | P::PayloadInvalid)
            | E::PayloadInvalid
            | E::Inbound(_)
            | E::Store(S::CodeInvalid | S::EventIdConflict | S::PayloadInvalid(_)) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            _ => return Self::internal(error),
        };
        let mut api = ApiError::new(status, code, "Suite request failed");
        match error {
            E::Unavailable => {
                api = api.with_retry_after(crate::domain::SUITE_UNAVAILABLE_RETRY_SECONDS)
            }
            E::RateLimited(seconds) | E::Store(S::ReplayTooSoon(seconds)) => {
                api = api.with_retry_after(seconds)
            }
            E::ReplayWindow { earliest, latest } => {
                api = api.with_details(std::collections::BTreeMap::from([
                    ("earliest".into(), serde_json::json!(earliest)),
                    ("latest".into(), serde_json::json!(latest)),
                ]))
            }
            _ => {}
        }
        api.into()
    }
}
#[derive(serde::Serialize, utoipa::ToSchema)]
pub(super) struct ErrorResponse {
    pub error: serde_json::Value,
}
