use std::{
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use aws_lc_rs::{
    encoding::AsDer as _,
    hmac,
    rsa::{KeyPair, KeySize},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};

use crate::{
    JwtClaims, MockOidcServer, rs256_token_with_key_id,
    security_probe::{
        Operation, OperationFixtures, RouteExclusion, SecurityTarget, bearer, operations,
        substitute, validate_exclusions,
    },
    unsigned_token,
};

const UNAUTHORIZED: u16 = 401;
const TOKEN_LIFETIME: Duration = Duration::from_secs(300);
const EXPIRED_BY_SECONDS: u64 = 300;
const PLACEHOLDER_PATH_VALUE: &str = "00000000-0000-0000-0000-000000000000";
const WRONG_AUDIENCE: &str = "baukit-auth-negative-other-audience";
const WRONG_ISSUER: &str = "https://auth-negative.invalid/realms/other";
const NOT_BEFORE_IN_SECONDS: u64 = 3600;
const GARBAGE_TOKEN: &str = "not-a-jwt";
const MALFORMED_AUTHORIZATION: &str = "Basic YXV0aC1uZWdhdGl2ZTpzZWNyZXQ=";

/// Invalid credentials sent to every secured operation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InvalidCredential {
    /// No `Authorization` header. The 401 must carry `WWW-Authenticate`.
    Missing,
    /// Signed by the issuer, but expired.
    Expired,
    /// Signed by the issuer for another audience.
    OtherAudience,
    /// Signed by a key the issuer never published, under the issuer's `kid`.
    UnknownKey,
    /// `alg: none` with an empty signature.
    AlgNone,
    /// Signed by the issuer's key, but naming another issuer.
    WrongIssuer,
    /// Signed by the issuer, with `nbf` an hour in the future.
    NotYetValid,
    /// HS256 keyed with the issuer's published JWK, under the issuer's `kid`.
    Hs256WithPublicKey,
    /// Signed by the issuer's key under a random `kid` the JWKS never published.
    RandomKeyId,
    /// A garbage bearer followed by a valid one in a second `Authorization` header.
    DuplicateAuthorization,
    /// A `Basic` credential instead of a bearer token.
    MalformedAuthorization,
}

impl InvalidCredential {
    /// All cases in the order the suite sends them.
    pub const ALL: [Self; 11] = [
        Self::Missing,
        Self::Expired,
        Self::OtherAudience,
        Self::UnknownKey,
        Self::AlgNone,
        Self::WrongIssuer,
        Self::NotYetValid,
        Self::Hs256WithPublicKey,
        Self::RandomKeyId,
        Self::DuplicateAuthorization,
        Self::MalformedAuthorization,
    ];

    fn name(self) -> &'static str {
        match self {
            Self::Missing => "missing token",
            Self::Expired => "expired token",
            Self::OtherAudience => "other-audience token",
            Self::UnknownKey => "unknown-key token",
            Self::AlgNone => "alg none token",
            Self::WrongIssuer => "wrong-issuer token",
            Self::NotYetValid => "not-yet-valid token",
            Self::Hs256WithPublicKey => "hs256 public-key token",
            Self::RandomKeyId => "random-kid token",
            Self::DuplicateAuthorization => "duplicate authorization",
            Self::MalformedAuthorization => "malformed authorization",
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
    /// Unsecured operations declared public, with their reasons.
    pub public: Vec<RouteExclusion>,
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
        for public in &self.public {
            writeln!(formatter, "public {}: {}", public.label(), public.reason)?;
        }
        Ok(())
    }
}

/// The issuer and audience the product trusts, plus explicit exclusions.
pub struct AuthNegativeCase<'a> {
    issuer: &'a MockOidcServer,
    audience: String,
    exclusions: Vec<RouteExclusion>,
    public: Vec<RouteExclusion>,
}

impl<'a> AuthNegativeCase<'a> {
    /// Mints tokens from `issuer`. `audience` is the one the product accepts.
    #[must_use]
    pub fn new(issuer: &'a MockOidcServer, audience: impl Into<String>) -> Self {
        Self {
            issuer,
            audience: audience.into(),
            exclusions: Vec::new(),
            public: Vec::new(),
        }
    }

    /// Leaves the secured operation `method path` out of the suite. The reason is printed on every run.
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

    /// Declares that `method path` is documented without security on purpose.
    ///
    /// Every operation without security must be declared, so an accidentally public route fails.
    #[must_use]
    pub fn public(
        mut self,
        method: &str,
        path: impl Into<String>,
        reason: impl Into<String>,
    ) -> Self {
        self.public.push(RouteExclusion::new(method, path, reason));
        self
    }
}

/// Sends every [`InvalidCredential`] to every operation that declares
/// security in `document` and collects each response that was not 401.
///
/// An operation is secured when its own `security`, or the document's
/// top-level `security` if it has none, lists at least one requirement and
/// no empty (anonymous) requirement. Every other operation must be declared
/// with [`AuthNegativeCase::public`]. Each probed operation also gets a valid
/// token, which must not answer 401, so a verifier that rejects everything
/// fails. Path parameters take the fixture value, or a nil UUID when the
/// fixture has none.
///
/// # Errors
///
/// Returns [`AuthNegativeError`] when the document or tokens cannot be prepared,
/// or listing each bad exclusion or public declaration, each undeclared
/// unsecured operation, each secured operation that accepted an invalid token
/// or answered without the expected `401`, and each one that rejected a valid token.
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
    let tokens = credentials(case).await.map_err(fail)?;
    let mut violations = Vec::new();
    validate_exclusions(&case.exclusions, &all, &mut violations);
    validate_exclusions(&case.public, &all, &mut violations);
    let mut probed = Vec::new();
    for operation in &all {
        let declared_public = case.public.iter().any(|public| public.matches(operation));
        if !is_secured(document, operation) {
            if !declared_public {
                violations.push(format!(
                    "{}: documented without security; declare it with .public(method, path, reason)",
                    operation.label()
                ));
            }
            continue;
        }
        if declared_public {
            violations.push(format!(
                "{}: declared public but documented as secured",
                operation.label()
            ));
        }
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
        public: case.public.clone(),
    })
}

/// Panics listing every secured operation that accepted an invalid credential.
///
/// # Panics
///
/// Panics with all violations found by [`check_auth_negative_suite`].
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

struct Credentials {
    invalid: Vec<(InvalidCredential, Vec<String>)>,
    valid: Vec<String>,
}

async fn probe(
    operation: &Operation,
    target: &SecurityTarget,
    fixtures: &OperationFixtures,
    credentials: &Credentials,
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
    for (credential, authorization) in &credentials.invalid {
        let request = fixtures.request(operation, &uri, authorization);
        match target.send(request).await {
            Ok(response) if response.status != UNAUTHORIZED => violations.push(format!(
                "{label}: {} got status {}, expected 401",
                credential.name(),
                response.status
            )),
            Ok(response)
                if *credential == InvalidCredential::Missing && !response.www_authenticate =>
            {
                violations.push(format!(
                    "{label}: {} got 401 without WWW-Authenticate",
                    credential.name()
                ));
            }
            Ok(_) => {}
            Err(message) => violations.push(format!(
                "{label}: {} request failed: {message}",
                credential.name()
            )),
        }
    }
    let request = fixtures.request(operation, &uri, &credentials.valid);
    match target.send(request).await {
        Ok(response) if response.status == UNAUTHORIZED => violations.push(format!(
            "{label}: valid token got 401, so the route rejects every caller (positive control)"
        )),
        Ok(_) => {}
        Err(message) => violations.push(format!("{label}: valid token request failed: {message}")),
    }
}

async fn credentials(case: &AuthNegativeCase<'_>) -> Result<Credentials, String> {
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
    let wrong_issuer = valid.clone().issuer(WRONG_ISSUER);
    let not_yet_valid = valid
        .clone()
        .claim("nbf", now.saturating_add(NOT_BEFORE_IN_SECONDS));
    let mint = |claims: &JwtClaims| issuer.mint(claims).map_err(|error| error.to_string());
    let valid_token = mint(&valid)?;
    let random_kid = issuer
        .mint_with_key_id(&valid, &uuid::Uuid::new_v4().to_string())
        .map_err(|error| error.to_string())?;
    let invalid = vec![
        (InvalidCredential::Missing, Vec::new()),
        (InvalidCredential::Expired, bearer(&mint(&expired)?)),
        (
            InvalidCredential::OtherAudience,
            bearer(&mint(&other_audience)?),
        ),
        (
            InvalidCredential::UnknownKey,
            bearer(&unknown_key_token(issuer, &valid)?),
        ),
        (
            InvalidCredential::AlgNone,
            bearer(&unsigned_token(&valid).map_err(|error| error.to_string())?),
        ),
        (
            InvalidCredential::WrongIssuer,
            bearer(&mint(&wrong_issuer)?),
        ),
        (
            InvalidCredential::NotYetValid,
            bearer(&mint(&not_yet_valid)?),
        ),
        (
            InvalidCredential::Hs256WithPublicKey,
            bearer(&hs256_public_key_token(issuer, &valid).await?),
        ),
        (InvalidCredential::RandomKeyId, bearer(&random_kid)),
        (
            InvalidCredential::DuplicateAuthorization,
            vec![
                format!("Bearer {GARBAGE_TOKEN}"),
                format!("Bearer {valid_token}"),
            ],
        ),
        (
            InvalidCredential::MalformedAuthorization,
            vec![MALFORMED_AUTHORIZATION.to_owned()],
        ),
    ];
    Ok(Credentials {
        invalid,
        valid: bearer(&valid_token),
    })
}

fn unknown_key_token(issuer: &MockOidcServer, claims: &JwtClaims) -> Result<String, String> {
    let key_pair = KeyPair::generate(KeySize::Rsa2048).map_err(|error| error.to_string())?;
    let der = key_pair.as_der().map_err(|error| error.to_string())?;
    let pem = pem::encode(&pem::Pem::new("PRIVATE KEY", der.as_ref()));
    let key_id = published_key_id(issuer)?;
    rs256_token_with_key_id(pem.as_bytes(), Some(&key_id), claims)
        .map_err(|error| error.to_string())
}

/// Algorithm confusion: a verifier that trusts the header's `alg` would check this HMAC
/// with the public key material it already holds.
async fn hs256_public_key_token(
    issuer: &MockOidcServer,
    claims: &JwtClaims,
) -> Result<String, String> {
    let key_id = published_key_id(issuer)?;
    let jwks: Value = crate::test_http_client()
        .build()
        .map_err(|error| error.to_string())?
        .get(issuer.jwks_url())
        .send()
        .await
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())?;
    let public_key = jwks
        .get("keys")
        .and_then(Value::as_array)
        .and_then(|keys| {
            keys.iter()
                .find(|key| key.get("kid").and_then(Value::as_str) == Some(key_id.as_str()))
        })
        .ok_or_else(|| "the JWKS does not publish the active kid".to_owned())?;
    let secret = serde_json::to_vec(public_key).map_err(|error| error.to_string())?;
    let header = json!({"alg": "HS256", "typ": "JWT", "kid": key_id});
    let encode = |value: &Value| {
        serde_json::to_vec(value)
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .map_err(|error| error.to_string())
    };
    let claims = serde_json::to_value(claims).map_err(|error| error.to_string())?;
    let signing_input = format!("{}.{}", encode(&header)?, encode(&claims)?);
    let key = hmac::Key::new(hmac::HMAC_SHA256, &secret);
    let signature = URL_SAFE_NO_PAD.encode(hmac::sign(&key, signing_input.as_bytes()));
    Ok(format!("{signing_input}.{signature}"))
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
