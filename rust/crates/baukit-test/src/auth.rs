use std::{fmt, time::Duration};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode, header},
};
use serde_json::{Value, json};
use tower::ServiceExt as _;

use crate::{MockOidcServer, authorization_header, unsigned_token};

const BODY_LIMIT: usize = 1024 * 1024;

/// Violations found while exercising protected-route error behavior.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthConformanceError {
    violations: Vec<String>,
}

impl AuthConformanceError {
    /// Returns violations in authentication-case order.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for AuthConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "authentication conformance failed:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AuthConformanceError {}

/// Checks that common invalid tokens receive the standard `401` envelope.
///
/// `router` must contain a protected GET route at `path`, use the supplied
/// mock issuer, and be wrapped in `baukit-http`'s request lifecycle. The helper
/// exercises expired, wrong-audience, wrong-issuer, and unsigned tokens.
pub async fn check_auth_router_conformance(
    router: &Router,
    path: &str,
    issuer: &MockOidcServer,
    audience: &str,
) -> Result<(), AuthConformanceError> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let valid = issuer
        .claims("conformance-subject", audience, Duration::from_secs(300))
        .map_err(|error| fixture_failure("valid claims", error))?;
    let cases = [
        (
            "expired",
            issuer
                .mint(&valid.clone().expires_at(now.saturating_sub(300)))
                .map_err(|error| fixture_failure("expired token", error))?,
        ),
        (
            "wrong-audience",
            issuer
                .mint(&valid.clone().audience("wrong-audience"))
                .map_err(|error| fixture_failure("wrong-audience token", error))?,
        ),
        (
            "wrong-issuer",
            issuer
                .mint(&valid.clone().issuer("https://wrong-issuer.invalid"))
                .map_err(|error| fixture_failure("wrong-issuer token", error))?,
        ),
        (
            "unsigned",
            unsigned_token(&valid).map_err(|error| fixture_failure("unsigned token", error))?,
        ),
    ];
    let mut violations = Vec::new();
    for (case, token) in cases {
        let request_id = format!("auth-conformance-{case}");
        let request = Request::builder()
            .uri(path)
            .header(
                header::AUTHORIZATION,
                authorization_header(&token).map_err(|error| AuthConformanceError {
                    violations: vec![format!(
                        "could not build {case} authorization header: {error}"
                    )],
                })?,
            )
            .header(baukit_http::X_REQUEST_ID, &request_id)
            .body(Body::empty())
            .map_err(|error| AuthConformanceError {
                violations: vec![format!("could not build {case} request: {error}")],
            })?;
        match router.clone().oneshot(request).await {
            Ok(response) => check_response(case, &request_id, response, &mut violations).await,
            Err(error) => violations.push(format!("{case} request failed: {error}")),
        }
    }
    if violations.is_empty() {
        Ok(())
    } else {
        Err(AuthConformanceError { violations })
    }
}

/// Panics when a protected route violates Baukit's invalid-token contract.
///
/// # Panics
///
/// Panics with all authentication contract violations or fixture failures.
pub async fn assert_auth_router_conformance(
    router: &Router,
    path: &str,
    issuer: &MockOidcServer,
    audience: &str,
) {
    if let Err(error) = check_auth_router_conformance(router, path, issuer, audience).await {
        panic!("{error}");
    }
}

fn fixture_failure(action: &str, error: impl fmt::Display) -> AuthConformanceError {
    AuthConformanceError {
        violations: vec![format!("could not create {action}: {error}")],
    }
}

async fn check_response(
    case: &str,
    request_id: &str,
    response: axum::response::Response,
    violations: &mut Vec<String>,
) {
    if response.status() != StatusCode::UNAUTHORIZED {
        violations.push(format!(
            "{case} token returned {}; expected 401",
            response.status()
        ));
    }
    if response
        .headers()
        .get(header::WWW_AUTHENTICATE)
        .and_then(|value| value.to_str().ok())
        != Some("Bearer")
    {
        violations.push(format!(
            "{case} response omitted `WWW-Authenticate: Bearer`"
        ));
    }
    let body = match to_bytes(response.into_body(), BODY_LIMIT).await {
        Ok(body) => body,
        Err(error) => {
            violations.push(format!("could not read {case} response body: {error}"));
            return;
        }
    };
    let actual: Value = match serde_json::from_slice(&body) {
        Ok(actual) => actual,
        Err(error) => {
            violations.push(format!("{case} response was not valid JSON: {error}"));
            return;
        }
    };
    let expected = json!({
        "error": {
            "code": "unauthenticated",
            "message": "Authentication is required",
            "request_id": request_id,
            "details": {}
        }
    });
    if actual != expected {
        violations.push(format!("{case} response envelope differed: {actual}"));
    }
}

#[cfg(test)]
mod tests {
    use axum::routing::get;
    use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};
    use baukit_http::{HttpOptions, finalize};

    use super::*;

    #[tokio::test]
    async fn real_auth_extractor_conforms() -> Result<(), Box<dyn std::error::Error>> {
        let issuer = MockOidcServer::start().await?;
        let config =
            OidcConfig::new(issuer.issuer(), "conformance-api")?.with_clock_skew(Duration::ZERO);
        let verifier = OidcVerifier::discover(config).await?;
        let router = Router::new()
            .route(
                "/protected",
                get(|principal: Principal| async move { principal.subject().to_owned() }),
            )
            .with_state(AuthState::new(verifier));
        let router = finalize(router, HttpOptions::default());
        check_auth_router_conformance(&router, "/protected", &issuer, "conformance-api").await?;
        Ok(())
    }
}
