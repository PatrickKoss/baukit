use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_lc_rs::{
    encoding::AsDer as _,
    rsa::{KeyPair, KeySize},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::Value;

use crate::{
    JwtClaims, MockOidcServer, rs256_token_with_key_id,
    security_probe::{
        Operation, OperationFixtures, RouteExclusion, SecurityTarget, operations, substitute,
        validate_exclusions,
    },
    unsigned_token,
};

const UNAUTHORIZED: u16 = 401;
const TOKEN_LIFETIME: Duration = Duration::from_secs(300);
const EXPIRED_BY_SECONDS: u64 = 300;
const PLACEHOLDER_PATH_VALUE: &str = "00000000-0000-0000-0000-000000000000";
const WRONG_AUDIENCE: &str = "baukit-auth-negative-other-audience";

/// Invalid credentials sent to every secured operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCredential {
    /// No `Authorization` header.
    Missing,
    /// Signed by the issuer, but expired.
    Expired,
    /// Signed by the issuer for another audience.
    OtherAudience,
    /// Signed by a key the issuer never published, under the issuer's `kid`.
    UnknownKey,
    /// `alg: none` with an empty signature.
    AlgNone,
}

impl InvalidCredential {
    /// All cases in the order the suite sends them.
    pub const ALL: [Self; 5] = [
        Self::Missing,
        Self::Expired,
        Self::OtherAudience,
        Self::UnknownKey,
        Self::AlgNone,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing token",
            Self::Expired => "expired token",
            Self::OtherAudience => "other-audience token",
            Self::UnknownKey => "unknown-key token",
            Self::AlgNone => "alg none token",
        }
    }
}

/// Operations a secured route answered with something other than 401.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthNegativeError {
    violations: Vec<String>,
}

impl AuthNegativeError {
    /// Returns one entry per operation and credential that was not rejected.
    #[must_use]
    pub fn violations(&self) -> &[String] {
        &self.violations
    }
}

impl fmt::Display for AuthNegativeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(formatter, "invalid credentials were not rejected with 401:")?;
        for violation in &self.violations {
            writeln!(formatter, "- {violation}")?;
        }
        Ok(())
    }
}

impl std::error::Error for AuthNegativeError {}

/// What a passing auth negative suite covered.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthNegativeReport {
    /// Secured operations probed, as `METHOD /path`.
    pub probed: Vec<String>,
    /// Secured operations left out, with their reasons.
    pub excluded: Vec<RouteExclusion>,
}

impl fmt::Display for AuthNegativeReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            formatter,
            "auth negative suite rejected {} invalid credential(s) on {} operation(s)",
            InvalidCredential::ALL.len(),
            self.probed.len()
        )?;
        for exclusion in &self.excluded {
            writeln!(
                formatter,
                "excluded {}: {}",
                exclusion.label(),
                exclusion.reason
            )?;
        }
        Ok(())
    }
}

/// The issuer and audience the product trusts, plus explicit exclusions.
pub struct AuthNegativeCase<'a> {
    issuer: &'a MockOidcServer,
    audience: String,
    exclusions: Vec<RouteExclusion>,
}

impl<'a> AuthNegativeCase<'a> {
    /// Mints tokens from `issuer`. `audience` is the one the product accepts.
    #[must_use]
    pub fn new(issuer: &'a MockOidcServer, audience: impl Into<String>) -> Self {
        Self {
            issuer,
            audience: audience.into(),
            exclusions: Vec::new(),
        }
    }

    /// Leaves `method path` out of the suite. The reason is printed on every run.
    #[must_use]
    pub fn exclude(
        mut self,
        method: &str,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        self.exclusions
            .push(RouteExclusion::new(method, path, reason));
        self
    }
}

/// Sends every [`InvalidCredential`] to every operation that declares
/// security in `document` and collects each response that was not 401.
///
/// An operation is secured when its own `security`, or the document's
/// top-level `security` if it has none, lists at least one requirement and
/// no empty (anonymous) requirement. Path parameters take the fixture value,
/// or a nil UUID when the fixture has none.
pub async fn check_auth_negative_suite(
    document: &Value,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    case: &AuthNegativeCase<'_>,
) -> Result<AuthNegativeReport, AuthNegativeError> {
    let fail = |message: String| AuthNegativeError {
        violations: vec![message],
    };
    let all = operations(document).map_err(fail)?;
    let tokens = invalid_tokens(case).map_err(fail)?;
    let mut violations = Vec::new();
    validate_exclusions(&case.exclusions, &all, &mut violations);
    let mut probed = Vec::new();
    for operation in all
        .iter()
        .filter(|operation| is_secured(document, operation))
    {
        if case
            .exclusions
            .iter()
            .any(|exclusion| exclusion.matches(operation))
        {
            continue;
        }
        probed.push(operation.label());
        probe(operation, target, fixtures, &tokens, &mut violations).await;
    }
    if !violations.is_empty() {
        return Err(AuthNegativeError { violations });
    }
    Ok(AuthNegativeReport {
        probed,
        excluded: case.exclusions.clone(),
    })
}

/// Panics listing every secured operation that accepted an invalid credential.
///
/// # Panics
///
/// Panics with all violations found by [`check_auth_negative_suite`].
#[expect(
    clippy::print_stdout,
    reason = "every run must show the probed count and each exclusion with its reason; libtest captures stdout per test"
)]
pub async fn assert_auth_negative_suite(
    document: &Value,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    case: &AuthNegativeCase<'_>,
) -> AuthNegativeReport {
    match check_auth_negative_suite(document, target, fixtures, case).await {
        Ok(report) => {
            println!("{report}");
            report
        }
        Err(error) => panic!("{error}"),
    }
}

fn is_secured(document: &Value, operation: &Operation) -> bool {
    let requirements = operation
        .item
        .get("security")
        .or_else(|| document.get("security"))
        .and_then(Value::as_array);
    requirements.is_some_and(|requirements| {
        !requirements.is_empty()
            && requirements
                .iter()
                .all(|requirement| requirement.as_object().is_some_and(|map| !map.is_empty()))
    })
}

async fn probe(
    operation: &Operation,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    tokens: &[(InvalidCredential, Option<String>)],
    violations: &mut Vec<String>,
) {
    let label = operation.label();
    let mut values = fixtures.path_values().clone();
    for name in operation.path_parameters() {
        values
            .entry(name.to_owned())
            .or_insert_with(|| PLACEHOLDER_PATH_VALUE.to_owned());
    }
    let uri = match substitute(&operation.path, &values) {
        Ok(uri) => uri,
        Err(message) => {
            violations.push(format!("{label}: {message}"));
            return;
        }
    };
    for (credential, token) in tokens {
        let request = fixtures.request(operation, &uri, token.as_deref());
        match target.send(request).await {
            Ok(response) if response.status == UNAUTHORIZED => {}
            Ok(response) => violations.push(format!(
                "{label}: {} got status {}, expected 401",
                credential.name(),
                response.status
            )),
            Err(message) => violations.push(format!(
                "{label}: {} request failed: {message}",
                credential.name()
            )),
        }
    }
}

fn invalid_tokens(
    case: &AuthNegativeCase<'_>,
) -> Result<Vec<(InvalidCredential, Option<String>)>, String> {
    let issuer = case.issuer;
    let valid = issuer
        .claims("auth-negative-subject", &case.audience, TOKEN_LIFETIME)
        .map_err(|error| format!("could not build claims: {error}"))?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs();
    let expired = valid
        .clone()
        .expires_at(now.saturating_sub(EXPIRED_BY_SECONDS));
    let other_audience = valid.clone().audience(WRONG_AUDIENCE);
    let mint = |claims: &JwtClaims| issuer.mint(claims).map_err(|error| error.to_string());
    Ok(vec![
        (InvalidCredential::Missing, None),
        (InvalidCredential::Expired, Some(mint(&expired)?)),
        (
            InvalidCredential::OtherAudience,
            Some(mint(&other_audience)?),
        ),
        (
            InvalidCredential::UnknownKey,
            Some(unknown_key_token(issuer, &valid)?),
        ),
        (
            InvalidCredential::AlgNone,
            Some(unsigned_token(&valid).map_err(|error| error.to_string())?),
        ),
    ])
}

fn unknown_key_token(issuer: &MockOidcServer, claims: &JwtClaims) -> Result<String, String> {
    let key_pair = KeyPair::generate(KeySize::Rsa2048).map_err(|error| error.to_string())?;
    let der = key_pair.as_der().map_err(|error| error.to_string())?;
    let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
    let key_id = published_key_id(issuer)?;
    rs256_token_with_key_id(pem.as_bytes(), Some(&key_id), claims)
        .map_err(|error| error.to_string())
}

fn published_key_id(issuer: &MockOidcServer) -> Result<String, String> {
    let token = issuer
        .mint(&JwtClaims::new())
        .map_err(|error| error.to_string())?;
    let header = token
        .split('.')
        .next()
        .ok_or_else(|| "minted token has no header".to_owned())?;
    let decoded = URL_SAFE_NO_PAD
        .decode(header)
        .map_err(|error| error.to_string())?;
    serde_json::from_slice::<Value>(&decoded)
        .map_err(|error| error.to_string())?
        .get("kid")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| "minted token header has no kid".to_owned())
}
