use std::sync::Arc;

use axum::{
    extract::{FromRef, FromRequestParts, Request, State},
    http::{header, request::Parts},
    middleware::Next,
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

/// Verifies a presented bearer credential and caches its [`Principal`].
///
/// Requests without an `Authorization` header continue without a principal so
/// anonymous routes and client-IP safety limits can handle them. A malformed,
/// invalid, or expired credential is rejected before inner middleware runs.
/// Place this middleware outside middleware that reads `Principal` from request
/// extensions.
pub async fn establish_principal(
    State(auth): State<AuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    if request.headers().get(header::AUTHORIZATION).is_none() {
        return next.run(request).await;
    }
    let Some(token) = bearer_token(request.headers()) else {
        return AuthRejection::InvalidToken.into_response();
    };
    match auth.verifier.verify(token).await {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(error) => log_verification_failure(error).into_response(),
    }
}

/// Public authentication or authorization rejection using Baukit's envelope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum AuthRejection {
    /// The request has no valid bearer identity.
    Unauthenticated,
    /// A bearer token was supplied but is invalid.
    InvalidToken,
    /// A bearer token was supplied but has expired.
    ExpiredToken,
    /// Verification could not complete because a backing service is unavailable.
    Unavailable,
    /// Verification could not complete because server or provider configuration is invalid.
    Misconfigured,
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
            Self::InvalidToken => invalid_token_response(INVALID_TOKEN_CHALLENGE),
            Self::ExpiredToken => invalid_token_response(EXPIRED_TOKEN_CHALLENGE),
            Self::Unavailable => baukit_http::ApiError::new(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "Identity verification is temporarily unavailable",
            )
            .with_retry_after(UNAVAILABLE_RETRY_AFTER_SECONDS)
            .with_header(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            )
            .into_response(),
            Self::Misconfigured => baukit_http::ApiError::new(
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "Internal server error",
            )
            .with_header(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            )
            .into_response(),
            Self::PermissionDenied => baukit_http::ApiError::permission_denied().into_response(),
        }
    }
}

const EXPIRED_TOKEN_CHALLENGE: &str =
    "Bearer error=\"invalid_token\", error_description=\"expired\"";
const INVALID_TOKEN_CHALLENGE: &str =
    "Bearer error=\"invalid_token\", error_description=\"invalid\"";
const UNAVAILABLE_RETRY_AFTER_SECONDS: u64 = 5;

fn invalid_token_response(challenge: &'static str) -> Response {
    let mut response = baukit_http::ApiError::unauthenticated().into_response();
    response.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        header::HeaderValue::from_static(challenge),
    );
    response
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
        let token = bearer_token(&parts.headers).ok_or(AuthRejection::Unauthenticated)?;
        let principal = AuthState::from_ref(state)
            .verifier
            .verify(token)
            .await
            .map_err(log_verification_failure)?;
        parts.extensions.insert(principal.clone());
        Ok(principal)
    }
}

fn bearer_token(headers: &axum::http::HeaderMap) -> Option<&str> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value.strip_prefix("Bearer ")?;
    (!token.is_empty() && !token.contains(char::is_whitespace)).then_some(token)
}

fn log_verification_failure(error: VerificationError) -> AuthRejection {
    if error.is_unavailable() || error.is_misconfigured() {
        tracing::error!(error = %error, "identity verification infrastructure failed");
    } else {
        tracing::debug!(error = %error, "bearer authentication failed");
    }
    AuthRejection::from(error)
}

impl From<VerificationError> for AuthRejection {
    fn from(error: VerificationError) -> Self {
        if error.is_unavailable() {
            Self::Unavailable
        } else if error.is_misconfigured() {
            Self::Misconfigured
        } else if matches!(error, VerificationError::Expired) {
            Self::ExpiredToken
        } else {
            Self::InvalidToken
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::ready,
        pin::Pin,
        sync::{Arc, Mutex},
    };

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        middleware,
        routing::get,
    };
    use serde_json::Value;
    use tower::ServiceExt as _;

    use super::*;

    #[derive(Clone)]
    struct AcceptFixture {
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl AcceptFixture {
        fn new() -> Self {
            Self {
                calls: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl IdentityVerifier for AcceptFixture {
        fn verify<'a>(
            &'a self,
            token: &'a str,
        ) -> Pin<Box<dyn Future<Output = Result<Principal, VerificationError>> + Send + 'a>>
        {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Box::pin(ready(match token {
                "valid" => Ok(Principal::new("fixture")),
                "expired" => Err(VerificationError::Expired),
                "unavailable" => Err(VerificationError::Unavailable),
                "misconfigured" => Err(VerificationError::DiscoveryIssuerMismatch),
                _ => Err(VerificationError::MalformedToken),
            }))
        }
    }

    async fn handler(principal: Principal) -> String {
        principal.subject().to_owned()
    }

    #[tokio::test]
    async fn extractor_accepts_valid_bearer_token() {
        let verifier = AcceptFixture::new();
        let response = Router::new()
            .route("/", get(handler))
            .with_state(AuthState::new(verifier))
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
        let verifier = AcceptFixture::new();
        let response = Router::new()
            .route("/", get(handler))
            .with_state(AuthState::new(verifier))
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
    async fn invalid_tokens_have_safe_specific_bearer_challenges() {
        for (token, challenge) in [
            (
                "expired",
                "Bearer error=\"invalid_token\", error_description=\"expired\"",
            ),
            (
                "invalid",
                "Bearer error=\"invalid_token\", error_description=\"invalid\"",
            ),
        ] {
            let response = Router::new()
                .route("/", get(handler))
                .with_state(AuthState::new(AcceptFixture::new()))
                .oneshot(
                    Request::builder()
                        .uri("/")
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::WWW_AUTHENTICATE], challenge);
            let json: Value = serde_json::from_slice(
                &to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("response body"),
            )
            .expect("JSON response");
            assert_eq!(json["error"]["code"], "unauthenticated");
        }
    }

    #[tokio::test]
    async fn unavailable_verification_returns_a_retryable_envelope_for_both_paths() {
        for use_middleware in [false, true] {
            let auth = AuthState::new(AcceptFixture::new());
            let mut app = Router::new()
                .route("/", get(handler))
                .with_state(auth.clone());
            if use_middleware {
                app = app.layer(middleware::from_fn_with_state(auth, establish_principal));
            }
            let response = app
                .oneshot(
                    Request::builder()
                        .header(header::AUTHORIZATION, "Bearer unavailable")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
            assert_eq!(response.headers()[header::RETRY_AFTER], "5");
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
            let body: Value = serde_json::from_slice(
                &to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("body"),
            )
            .expect("JSON");
            assert_eq!(body["error"]["code"], "unavailable");
            assert_eq!(
                body["error"]["message"],
                "Identity verification is temporarily unavailable"
            );
            assert!(body["error"]["requestId"].is_string());
            assert_eq!(body["error"]["details"], serde_json::json!({}));
        }
    }

    #[tokio::test]
    async fn misconfigured_verification_returns_500_without_retry_for_both_paths() {
        for use_middleware in [false, true] {
            let auth = AuthState::new(AcceptFixture::new());
            let mut app = Router::new()
                .route("/", get(handler))
                .with_state(auth.clone());
            if use_middleware {
                app = app.layer(middleware::from_fn_with_state(auth, establish_principal));
            }
            let response = app
                .oneshot(
                    Request::builder()
                        .header(header::AUTHORIZATION, "Bearer misconfigured")
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
            assert!(!response.headers().contains_key(header::RETRY_AFTER));
            assert!(!response.headers().contains_key(header::WWW_AUTHENTICATE));
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body: Value = serde_json::from_slice(
                &to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("body"),
            )
            .expect("JSON");
            assert_eq!(body["error"]["code"], "internal");
            assert_eq!(body["error"]["message"], "Internal server error");
        }
    }

    #[tokio::test]
    async fn middleware_allows_missing_credentials_without_a_principal() {
        async fn anonymous(principal: Option<axum::Extension<Principal>>) -> StatusCode {
            if principal.is_none() {
                StatusCode::NO_CONTENT
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            }
        }

        let app = Router::new()
            .route("/", get(anonymous))
            .layer(middleware::from_fn_with_state(
                AuthState::new(AcceptFixture::new()),
                establish_principal,
            ));
        let response = app
            .oneshot(Request::new(Body::empty()))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn middleware_rejects_malformed_invalid_and_expired_credentials() {
        for (authorization, challenge) in [
            (
                "Basic opaque",
                "Bearer error=\"invalid_token\", error_description=\"invalid\"",
            ),
            (
                "Bearer invalid",
                "Bearer error=\"invalid_token\", error_description=\"invalid\"",
            ),
            (
                "Bearer expired",
                "Bearer error=\"invalid_token\", error_description=\"expired\"",
            ),
        ] {
            let app = Router::new()
                .route("/", get(|| async { StatusCode::NO_CONTENT }))
                .layer(middleware::from_fn_with_state(
                    AuthState::new(AcceptFixture::new()),
                    establish_principal,
                ));
            let response = app
                .oneshot(
                    Request::builder()
                        .header(header::AUTHORIZATION, authorization)
                        .body(Body::empty())
                        .expect("request"),
                )
                .await
                .expect("response");
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(response.headers()[header::WWW_AUTHENTICATE], challenge);
            let json: Value = serde_json::from_slice(
                &to_bytes(response.into_body(), usize::MAX)
                    .await
                    .expect("response body"),
            )
            .expect("JSON response");
            assert_eq!(json["error"]["code"], "unauthenticated");
        }
    }

    #[tokio::test]
    async fn extractor_reuses_the_middleware_principal() {
        let verifier = AcceptFixture::new();
        let app = Router::new()
            .route("/", get(handler))
            .with_state(AuthState::new(verifier.clone()))
            .layer(middleware::from_fn_with_state(
                AuthState::new(verifier.clone()),
                establish_principal,
            ));
        let response = app
            .oneshot(
                Request::builder()
                    .header(header::AUTHORIZATION, "Bearer valid")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(verifier.calls(), 1);
    }

    #[test]
    fn verification_logs_never_contain_the_bearer_token() {
        #[derive(Clone)]
        struct Captured(Arc<Mutex<Vec<u8>>>);

        impl std::io::Write for Captured {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().expect("capture lock").write(bytes)
            }

            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let output = Arc::new(Mutex::new(Vec::new()));
        let writer = Captured(Arc::clone(&output));
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish();
        let secret = "sensitive.token.value";

        let rejection = tracing::subscriber::with_default(subscriber, || {
            log_verification_failure(VerificationError::MalformedToken)
        });
        assert_eq!(rejection, AuthRejection::InvalidToken);

        let output = String::from_utf8(output.lock().expect("capture lock").clone())
            .expect("UTF-8 tracing output");
        assert!(output.contains("bearer authentication failed"));
        assert!(!output.contains(secret));

        let infrastructure = Arc::new(Mutex::new(Vec::new()));
        let writer = Captured(infrastructure.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_target(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert_eq!(
                AuthRejection::from(VerificationError::Unavailable),
                AuthRejection::Unavailable
            );
            assert!(infrastructure.lock().expect("capture lock").is_empty());
            assert_eq!(
                log_verification_failure(VerificationError::Unavailable),
                AuthRejection::Unavailable
            );
            assert_eq!(
                log_verification_failure(VerificationError::DiscoveryIssuerMismatch),
                AuthRejection::Misconfigured
            );
        });
        let output = String::from_utf8(infrastructure.lock().expect("capture lock").clone())
            .expect("UTF-8 tracing output");
        assert!(output.contains("ERROR identity verification infrastructure failed"));
        assert!(output.contains("error=identity verification is temporarily unavailable"));
        assert!(output.contains("error=OIDC discovery issuer did not match configuration"));
        assert!(!output.contains("bearer authentication failed"));
        assert!(!output.contains(secret));
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
