use std::sync::Arc;

use axum::{
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts},
    response::{IntoResponse, Response},
};

use crate::{IdentityVerifier, Principal, VerificationError};

/// Cloneable Axum state holding a provider-neutral identity verifier.
#[derive(Clone)]
pub struct AuthState {
    verifier: Arc<dyn IdentityVerifier>,
}

impl AuthState {
    /// Creates extractor state from any identity-verifier adapter.
    #[must_use]
    pub fn new(verifier: impl IdentityVerifier + 'static) -> Self {
        Self {
            verifier: Arc::new(verifier),
        }
    }

    /// Creates extractor state from an already shared verifier adapter.
    #[must_use]
    pub fn from_shared(verifier: Arc<dyn IdentityVerifier>) -> Self {
        Self { verifier }
    }
}

impl std::fmt::Debug for AuthState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("AuthState").finish_non_exhaustive()
    }
}

/// Public authentication or authorization rejection using Baukit's envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthRejection {
    /// The request has no valid bearer identity.
    Unauthenticated,
    /// The identity is valid but is not allowed to perform the operation.
    PermissionDenied,
}

impl AuthRejection {
    /// Creates a `403 permission_denied` rejection for product authorization checks.
    #[must_use]
    pub const fn permission_denied() -> Self {
        Self::PermissionDenied
    }
}

impl IntoResponse for AuthRejection {
    fn into_response(self) -> Response {
        match self {
            Self::Unauthenticated => {
                let mut response = baukit_http::ApiError::unauthenticated().into_response();
                response.headers_mut().insert(
                    header::WWW_AUTHENTICATE,
                    header::HeaderValue::from_static("Bearer"),
                );
                response
            }
            Self::PermissionDenied => baukit_http::ApiError::permission_denied().into_response(),
        }
    }
}

impl<S> FromRequestParts<S> for Principal
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthRejection;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Some(principal) = parts.extensions.get::<Principal>() {
            return Ok(principal.clone());
        }
        let token = bearer_token(parts).ok_or(AuthRejection::Unauthenticated)?;
        let principal = AuthState::from_ref(state)
            .verifier
            .verify(token)
            .await
            .map_err(log_verification_failure)?;
        parts.extensions.insert(principal.clone());
        Ok(principal)
    }
}

fn bearer_token(parts: &Parts) -> Option<&str> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    (!token.is_empty() && !token.contains(char::is_whitespace)).then_some(token)
}

fn log_verification_failure(error: VerificationError) -> AuthRejection {
    tracing::debug!(error = %error, "bearer authentication failed");
    AuthRejection::Unauthenticated
}

#[cfg(test)]
mod tests {
    use std::{future::ready, pin::Pin};

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::get,
    };
    use serde_json::Value;
    use tower::ServiceExt as _;

    use super::*;

    struct AcceptFixture;

    impl IdentityVerifier for AcceptFixture {
        fn verify<'a>(
            &'a self,
            token: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>>
        {
            Box::pin(ready(if token == "valid" {
                Ok(Principal::new("fixture"))
            } else {
                Err(VerificationError::MalformedToken)
            }))
        }
    }

    async fn handler(principal: Principal) -> String {
        principal.subject().to_owned()
    }

    #[tokio::test]
    async fn extractor_accepts_valid_bearer_token() {
        let response = Router::new()
            .route("/", get(handler))
            .with_state(AuthState::new(AcceptFixture))
            .oneshot(
                Request::builder()
                    .uri("/")
                    .header(header::AUTHORIZATION, "Bearer valid")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn extractor_uses_standard_unauthenticated_envelope() {
        let response = Router::new()
            .route("/", get(handler))
            .with_state(AuthState::new(AcceptFixture))
            .oneshot(
                Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(response.headers()[header::WWW_AUTHENTICATE], "Bearer");
        let json: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body"),
        )
        .expect("JSON response");
        assert_eq!(json["error"]["code"], "unauthenticated");
    }

    #[tokio::test]
    async fn authorization_rejection_uses_standard_permission_envelope() {
        let response = AuthRejection::permission_denied().into_response();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let json: Value = serde_json::from_slice(
            &to_bytes(response.into_body(), usize::MAX)
                .await
                .expect("response body"),
        )
        .expect("JSON response");
        assert_eq!(json["error"]["code"], "permission_denied");
    }
}
