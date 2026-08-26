use std::{
    collections::BTreeMap,
    io,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderValue, StatusCode, header::InvalidHeaderValue},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::{hmac, rand::SystemRandom, signature};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::{net::TcpListener, task::JoinHandle, time::sleep};

const REALM_PATH: &str = "/realms/baukit-test";
const KEY_1_ID: &str = "baukit-test-key-1";
const KEY_2_ID: &str = "baukit-test-key-2";
const KEY_1_MODULUS: &str = "8XoYIfBj-BNazQ5v2ueAX9pM0_bjXiIuseeA5nDQTkKtfKjMLXxSgdGrRlyf7SyuZb48JsvJUF2O1rcvoXxIuRXGjImVbWeBlfY3f2xNuUv9g3WTnEvcTzLZCkz0CCiXdJ7ntk0DcQe4Eh3cNe0zSJ2yEOxbzzWtk9Wzh0LY7s1g_aAc0jTak0KQpflKWyRRAK-KQyZlklij0TJkhM4VyZMVL_wgrJe3DIgpzfz7SG9yfouU9ut7QITYqXUCkuYY6v2WlvJi2AFlA4daGOitmL3f2ecPRcjnoK818jo6kFlpwWXM5Lp8iv4eR9gJEt7t7QbtNG0okTpoBH7caU9eaQ";
const KEY_2_MODULUS: &str = "0fCAqd3b6BRLfybKKnuefZtfR8O1CU1Kwe5wKw9aY_VEiTTM0w90qV_h9MNiQMjEkSlVRzmPR7Tccvy5PUIbFrS9-egGL7cd7xA-p9Ya-i71dDp8F1a6XKq9rwVrZPXN9Kq-Eot3NA3oVX3Ts9XBTJDT2XitVprpIyocjdHrA6LalMUY9vTY7ztJDtT3f49Hayc8skHd3HqGMXXc2ME8U7RLAyiHigyBzaY_TDNnK2RFYgvvxjb1nG_UaOUFGorkEb7ePxbcnsaJKSoHYPxjuP9FWL9FUt-FMOcMpwA7iqhppkoOWfuN-kzTmCDQgQopeqI-aiTZYMx0IjmDxAa-Ww";
const RSA_EXPONENT: &str = "AQAB";
const KEY_1_PEM: &[u8] = include_bytes!("jwt/fixtures/oidc-key-1.pem");
const KEY_2_PEM: &[u8] = include_bytes!("jwt/fixtures/oidc-key-2.pem");

/// Configurable registered and custom claims for authentication fixtures.
///
/// Omitted values are not serialized, making it possible to exercise handlers
/// that accept or reject each registered claim independently.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct JwtClaims {
    /// Subject (`sub`) identifying the test principal.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sub: Option<String>,
    /// Issuer (`iss`) expected by the test application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub iss: Option<String>,
    /// Audience (`aud`) expected by the test application.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Expiry (`exp`) as seconds since the Unix epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exp: Option<u64>,
    /// Not-before time (`nbf`) as seconds since the Unix epoch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nbf: Option<u64>,
    /// Provider-shaped claims used to test configured principal mappings.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

impl JwtClaims {
    /// Creates an empty claims set.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            sub: None,
            iss: None,
            aud: None,
            exp: None,
            nbf: None,
            extra: BTreeMap::new(),
        }
    }

    /// Sets the subject claim.
    #[must_use]
    pub fn subject(mut self, subject: impl Into<String>) -> Self {
        self.sub = Some(subject.into());
        self
    }

    /// Sets the issuer claim.
    #[must_use]
    pub fn issuer(mut self, issuer: impl Into<String>) -> Self {
        self.iss = Some(issuer.into());
        self
    }

    /// Sets the audience claim.
    #[must_use]
    pub fn audience(mut self, audience: impl Into<String>) -> Self {
        self.aud = Some(audience.into());
        self
    }

    /// Sets the expiry claim in Unix-epoch seconds.
    #[must_use]
    pub const fn expires_at(mut self, expiry: u64) -> Self {
        self.exp = Some(expiry);
        self
    }

    /// Sets the not-before claim in Unix-epoch seconds.
    #[must_use]
    pub const fn not_before(mut self, not_before: u64) -> Self {
        self.nbf = Some(not_before);
        self
    }

    /// Adds a custom top-level claim.
    #[must_use]
    pub fn claim(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.extra.insert(name.into(), value.into());
        self
    }
}

/// Encodes claims as an HS256 JWT with a test-only shared secret.
pub fn hs256_token(secret: &[u8], claims: &JwtClaims) -> Result<String, JwtFixtureError> {
    encode_token(json!({"alg": "HS256", "typ": "JWT"}), claims, |message| {
        let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
        Ok(hmac::sign(&key, message).as_ref().to_vec())
    })
}

/// Encodes claims as an RS256 JWT using a PEM-encoded RSA private key.
pub fn rs256_token(private_key_pem: &[u8], claims: &JwtClaims) -> Result<String, JwtFixtureError> {
    rs256_token_with_key_id(private_key_pem, None, claims)
}

/// Encodes claims as an RS256 JWT with an optional JWKS key identifier.
pub fn rs256_token_with_key_id(
    private_key_pem: &[u8],
    key_id: Option<&str>,
    claims: &JwtClaims,
) -> Result<String, JwtFixtureError> {
    let pem = pem::parse(private_key_pem)?;
    let key_pair = match pem.tag() {
        "PRIVATE KEY" => signature::RsaKeyPair::from_pkcs8(pem.contents()),
        "RSA PRIVATE KEY" => signature::RsaKeyPair::from_der(pem.contents()),
        _ => return Err(JwtFixtureError::UnsupportedPemTag(pem.tag().to_owned())),
    }
    .map_err(|error| JwtFixtureError::InvalidRsaKey(error.to_string()))?;
    let mut header = json!({"alg": "RS256", "typ": "JWT"});
    if let Some(key_id) = key_id {
        header["kid"] = Value::String(key_id.to_owned());
    }
    encode_token(header, claims, |message| {
        let mut output = vec![0; key_pair.public().modulus_len()];
        key_pair
            .sign(
                &signature::RSA_PKCS1_SHA256,
                &SystemRandom::new(),
                message,
                &mut output,
            )
            .map_err(|_| JwtFixtureError::Signing)?;
        Ok(output)
    })
}

/// Encodes claims in an unsigned JWT using the forbidden `none` algorithm.
pub fn unsigned_token(claims: &JwtClaims) -> Result<String, JwtFixtureError> {
    let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&json!({"alg": "none"}))?);
    let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims)?);
    Ok(format!("{header}.{claims}."))
}

fn encode_token<F>(header: Value, claims: &JwtClaims, sign: F) -> Result<String, JwtFixtureError>
where
    F: FnOnce(&[u8]) -> Result<Vec<u8>, JwtFixtureError>,
{
    let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header)?);
    let claims = URL_SAFE_NO_PAD.encode(serde_json::to_vec(claims)?);
    let signing_input = format!("{header}.{claims}");
    let signature = URL_SAFE_NO_PAD.encode(sign(signing_input.as_bytes())?);
    Ok(format!("{signing_input}.{signature}"))
}

/// Creates an HTTP `Authorization` header value for a generated JWT.
pub fn authorization_header(token: &str) -> Result<HeaderValue, InvalidHeaderValue> {
    HeaderValue::from_str(&format!("Bearer {token}"))
}

/// In-process OIDC discovery and JWKS server with a rotating RS256 signer.
pub struct MockOidcServer {
    base_url: String,
    state: MockState,
    task: JoinHandle<io::Result<()>>,
}

impl MockOidcServer {
    /// Starts a mock issuer on an ephemeral loopback port.
    pub async fn start() -> Result<Self, JwtFixtureError> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let address = listener.local_addr()?;
        let base_url = format!("http://{address}");
        let issuer = format!("{base_url}{REALM_PATH}");
        let state = MockState::new(issuer);
        let router = Router::new()
            .route(
                &format!("{REALM_PATH}/.well-known/openid-configuration"),
                get(discovery),
            )
            .route(
                &format!("{REALM_PATH}/protocol/openid-connect/certs"),
                get(jwks),
            )
            .route(
                &format!("{REALM_PATH}/protocol/openid-connect/token"),
                post(refresh_token),
            )
            .with_state(state.clone());
        let task = tokio::spawn(async move { axum::serve(listener, router).await });
        Ok(Self {
            base_url,
            state,
            task,
        })
    }

    /// Returns the realm issuer URL advertised through discovery.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.state.issuer
    }

    /// Returns the server origin without the realm path.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Builds claims with this issuer and an expiry relative to now.
    pub fn claims(
        &self,
        subject: impl Into<String>,
        audience: impl Into<String>,
        lifetime: Duration,
    ) -> Result<JwtClaims, JwtFixtureError> {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
        Ok(JwtClaims::new()
            .subject(subject)
            .issuer(self.issuer())
            .audience(audience)
            .expires_at(now.saturating_add(lifetime.as_secs())))
    }

    /// Mints an RS256 token with the active signing key and its `kid` header.
    pub fn mint(&self, claims: &JwtClaims) -> Result<String, JwtFixtureError> {
        mint_with_active_key(&self.state, claims)
    }

    /// Creates an access/refresh token pair whose access token has the requested lifetime.
    pub fn issue_session(
        &self,
        subject: impl Into<String>,
        audience: impl Into<String>,
        access_token_lifetime: Duration,
    ) -> Result<MockOidcSession, JwtFixtureError> {
        let sequence = self
            .state
            .next_session
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        let refresh_token = format!("baukit-test-refresh-{sequence}");
        let session = MockSession {
            subject: subject.into(),
            audience: audience.into(),
            access_token_lifetime,
            refresh_rejected: false,
            revoked: false,
        };
        let access_token = mint_session_access_token(&self.state, &session)?;
        self.state
            .sessions
            .write()
            .expect("mock session lock")
            .insert(refresh_token.clone(), session);
        Ok(MockOidcSession {
            access_token,
            refresh_token,
            expires_in: access_token_lifetime,
        })
    }

    /// Requests a new access token from the mock token endpoint.
    ///
    /// Rejected or revoked refresh tokens return [`JwtFixtureError::RefreshRejected`],
    /// providing a deterministic terminal-refresh fixture.
    pub async fn refresh_session(
        &self,
        refresh_token: &str,
    ) -> Result<MockOidcSession, JwtFixtureError> {
        #[derive(Serialize)]
        struct Request<'a> {
            grant_type: &'static str,
            refresh_token: &'a str,
        }

        let body = serde_html_form::to_string(Request {
            grant_type: "refresh_token",
            refresh_token,
        })?;
        let response = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()?
            .post(format!(
                "{}{REALM_PATH}/protocol/openid-connect/token",
                self.base_url
            ))
            .header("content-type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?;
        let status = response.status();
        let body: TokenEndpointResponse = response.json().await?;
        if status.is_success() {
            Ok(MockOidcSession {
                access_token: body
                    .access_token
                    .ok_or(JwtFixtureError::InvalidTokenResponse)?,
                refresh_token: body
                    .refresh_token
                    .ok_or(JwtFixtureError::InvalidTokenResponse)?,
                expires_in: Duration::from_secs(
                    body.expires_in
                        .ok_or(JwtFixtureError::InvalidTokenResponse)?,
                ),
            })
        } else {
            Err(JwtFixtureError::RefreshRejected {
                code: body.error.unwrap_or_else(|| "invalid_grant".to_owned()),
            })
        }
    }

    /// Makes a session's refresh token return terminal `invalid_grant`.
    #[must_use]
    pub fn reject_refresh(&self, refresh_token: &str) -> bool {
        update_session(&self.state, refresh_token, |session| {
            session.refresh_rejected = true;
        })
    }

    /// Revokes a mock session so subsequent refresh attempts are terminal.
    #[must_use]
    pub fn revoke_session(&self, refresh_token: &str) -> bool {
        update_session(&self.state, refresh_token, |session| {
            session.revoked = true;
        })
    }

    /// Delays subsequent refresh responses so clients can prove single-flight behavior.
    pub fn set_refresh_delay(&self, delay: Duration) {
        self.state.refresh_delay_millis.store(
            delay.as_millis().try_into().unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }

    /// Returns how many refresh-token requests the fixture has served.
    #[must_use]
    pub fn refresh_request_count(&self) -> usize {
        self.state.refresh_requests.load(Ordering::SeqCst)
    }

    /// Rotates signing and published verification material to a new key.
    pub fn rotate_signing_key(&self) {
        self.state.active_key.store(2, Ordering::SeqCst);
        *self.state.keys.write().expect("mock JWKS lock") = vec![jwk(KEY_2_ID, KEY_2_MODULUS)];
    }

    /// Delays subsequent JWKS responses to exercise verifier timeouts.
    pub fn set_jwks_delay(&self, delay: Duration) {
        self.state.jwks_delay_millis.store(
            delay.as_millis().try_into().unwrap_or(u64::MAX),
            Ordering::SeqCst,
        );
    }

    /// Returns how many JWKS requests the fixture has served.
    #[must_use]
    pub fn jwks_request_count(&self) -> usize {
        self.state.jwks_requests.load(Ordering::SeqCst)
    }
}

impl Drop for MockOidcServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct MockState {
    issuer: String,
    keys: Arc<RwLock<Vec<Value>>>,
    active_key: Arc<AtomicUsize>,
    jwks_delay_millis: Arc<AtomicU64>,
    jwks_requests: Arc<AtomicUsize>,
    sessions: Arc<RwLock<BTreeMap<String, MockSession>>>,
    next_session: Arc<AtomicU64>,
    refresh_delay_millis: Arc<AtomicU64>,
    refresh_requests: Arc<AtomicUsize>,
}

impl MockState {
    fn new(issuer: String) -> Self {
        Self {
            issuer,
            keys: Arc::new(RwLock::new(vec![jwk(KEY_1_ID, KEY_1_MODULUS)])),
            active_key: Arc::new(AtomicUsize::new(1)),
            jwks_delay_millis: Arc::new(AtomicU64::new(0)),
            jwks_requests: Arc::new(AtomicUsize::new(0)),
            sessions: Arc::new(RwLock::new(BTreeMap::new())),
            next_session: Arc::new(AtomicU64::new(0)),
            refresh_delay_millis: Arc::new(AtomicU64::new(0)),
            refresh_requests: Arc::new(AtomicUsize::new(0)),
        }
    }
}

/// Access and refresh credentials issued by [`MockOidcServer`].
///
/// This type deliberately does not implement `Debug`, avoiding accidental
/// credential disclosure in assertion or tracing output.
#[derive(Clone)]
pub struct MockOidcSession {
    access_token: String,
    refresh_token: String,
    expires_in: Duration,
}

impl MockOidcSession {
    /// Returns the short-lived bearer access token.
    #[must_use]
    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Returns the refresh credential used by the mock token endpoint.
    #[must_use]
    pub fn refresh_token(&self) -> &str {
        &self.refresh_token
    }

    /// Returns the advertised access-token lifetime.
    #[must_use]
    pub const fn expires_in(&self) -> Duration {
        self.expires_in
    }
}

#[derive(Clone)]
struct MockSession {
    subject: String,
    audience: String,
    access_token_lifetime: Duration,
    refresh_rejected: bool,
    revoked: bool,
}

#[derive(Deserialize, Serialize)]
struct TokenEndpointResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_in: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

async fn discovery(State(state): State<MockState>) -> Json<Value> {
    Json(json!({
        "issuer": state.issuer,
        "jwks_uri": format!("{}{REALM_PATH}/protocol/openid-connect/certs", origin(&state.issuer)),
        "authorization_endpoint": format!("{}/protocol/openid-connect/auth", state.issuer),
        "token_endpoint": format!("{}/protocol/openid-connect/token", state.issuer),
        "id_token_signing_alg_values_supported": ["RS256"]
    }))
}

async fn jwks(State(state): State<MockState>) -> Json<Value> {
    state.jwks_requests.fetch_add(1, Ordering::SeqCst);
    let delay = state.jwks_delay_millis.load(Ordering::SeqCst);
    if delay > 0 {
        sleep(Duration::from_millis(delay)).await;
    }
    let keys = state.keys.read().expect("mock JWKS lock").clone();
    Json(json!({"keys": keys}))
}

async fn refresh_token(State(state): State<MockState>, body: Bytes) -> Response {
    #[derive(Deserialize)]
    struct Request {
        grant_type: String,
        refresh_token: String,
    }

    state.refresh_requests.fetch_add(1, Ordering::SeqCst);
    let delay = state.refresh_delay_millis.load(Ordering::SeqCst);
    if delay > 0 {
        sleep(Duration::from_millis(delay)).await;
    }
    let Ok(request) = serde_html_form::from_bytes::<Request>(&body) else {
        return token_error(StatusCode::BAD_REQUEST, "invalid_request");
    };
    if request.grant_type != "refresh_token" {
        return token_error(StatusCode::BAD_REQUEST, "unsupported_grant_type");
    }
    let session = state
        .sessions
        .read()
        .expect("mock session lock")
        .get(&request.refresh_token)
        .cloned();
    let Some(session) = session else {
        return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
    };
    if session.refresh_rejected || session.revoked {
        return token_error(StatusCode::BAD_REQUEST, "invalid_grant");
    }
    match mint_session_access_token(&state, &session) {
        Ok(access_token) => (
            StatusCode::OK,
            Json(TokenEndpointResponse {
                access_token: Some(access_token),
                refresh_token: Some(request.refresh_token),
                expires_in: Some(session.access_token_lifetime.as_secs()),
                token_type: Some("Bearer".to_owned()),
                error: None,
            }),
        )
            .into_response(),
        Err(_) => token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error"),
    }
}

fn token_error(status: StatusCode, code: &str) -> Response {
    (
        status,
        Json(TokenEndpointResponse {
            access_token: None,
            refresh_token: None,
            expires_in: None,
            token_type: None,
            error: Some(code.to_owned()),
        }),
    )
        .into_response()
}

fn mint_session_access_token(
    state: &MockState,
    session: &MockSession,
) -> Result<String, JwtFixtureError> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let claims = JwtClaims::new()
        .subject(&session.subject)
        .issuer(&state.issuer)
        .audience(&session.audience)
        .expires_at(now.saturating_add(session.access_token_lifetime.as_secs()));
    mint_with_active_key(state, &claims)
}

fn mint_with_active_key(state: &MockState, claims: &JwtClaims) -> Result<String, JwtFixtureError> {
    match state.active_key.load(Ordering::SeqCst) {
        1 => rs256_token_with_key_id(KEY_1_PEM, Some(KEY_1_ID), claims),
        2 => rs256_token_with_key_id(KEY_2_PEM, Some(KEY_2_ID), claims),
        _ => Err(JwtFixtureError::InvalidActiveKey),
    }
}

fn update_session(
    state: &MockState,
    refresh_token: &str,
    update: impl FnOnce(&mut MockSession),
) -> bool {
    let mut sessions = state.sessions.write().expect("mock session lock");
    if let Some(session) = sessions.get_mut(refresh_token) {
        update(session);
        true
    } else {
        false
    }
}

fn origin(issuer: &str) -> &str {
    issuer.strip_suffix(REALM_PATH).unwrap_or(issuer)
}

fn jwk(key_id: &str, modulus: &str) -> Value {
    json!({
        "kty": "RSA",
        "kid": key_id,
        "use": "sig",
        "key_ops": ["verify"],
        "alg": "RS256",
        "n": modulus,
        "e": RSA_EXPONENT
    })
}

/// Failure while constructing or hosting a JWT fixture.
#[derive(Debug, Error)]
pub enum JwtFixtureError {
    /// Claims or headers could not be serialized.
    #[error("could not serialize JWT fixture: {0}")]
    Json(#[from] serde_json::Error),
    /// A private key was not valid PEM.
    #[error("could not parse JWT fixture private key: {0}")]
    Pem(#[from] pem::PemError),
    /// A PEM block had an unsupported label.
    #[error("unsupported JWT fixture PEM label `{0}`")]
    UnsupportedPemTag(String),
    /// RSA private-key validation failed.
    #[error("invalid JWT fixture RSA key: {0}")]
    InvalidRsaKey(String),
    /// Cryptographic signing failed.
    #[error("could not sign JWT fixture")]
    Signing,
    /// The mock server could not bind or inspect its socket.
    #[error("mock OIDC server I/O failed: {0}")]
    Io(#[from] io::Error),
    /// A mock form request could not be encoded or decoded.
    #[error("mock OIDC form was invalid: {0}")]
    Form(#[from] serde_html_form::ser::Error),
    /// A mock OIDC HTTP request or response failed.
    #[error("mock OIDC HTTP request failed: {0}")]
    Http(#[from] reqwest::Error),
    /// The mock token endpoint rejected refresh as a terminal auth failure.
    #[error("mock OIDC refresh was rejected with `{code}`")]
    RefreshRejected {
        /// Stable OAuth error code returned by the fixture.
        code: String,
    },
    /// The mock token endpoint returned an incomplete success response.
    #[error("mock OIDC token response was incomplete")]
    InvalidTokenResponse,
    /// The system clock was before the Unix epoch.
    #[error("system clock is before the Unix epoch: {0}")]
    Clock(#[from] std::time::SystemTimeError),
    /// The fixture selected a nonexistent signing key.
    #[error("mock OIDC server selected an invalid active key")]
    InvalidActiveKey,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use baukit_auth::{OidcConfig, OidcVerifier, VerificationError};

    use super::*;

    #[test]
    fn creates_hs256_fixture_with_configured_claims() -> Result<(), Box<dyn std::error::Error>> {
        let claims = JwtClaims::new()
            .subject("user-123")
            .issuer("fixture")
            .audience("api")
            .expires_at(4_102_444_800);
        let token = hs256_token(b"fixture-secret", &claims)?;
        assert_eq!(token.split('.').count(), 3);
        assert_eq!(
            authorization_header(&token)?.to_str().expect("header text"),
            format!("Bearer {token}")
        );
        Ok(())
    }

    #[tokio::test]
    async fn mock_server_discovers_verifies_caches_and_rotates()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = MockOidcServer::start().await?;
        let verifier = OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?;
        let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
        assert_eq!(
            verifier.verify(&server.mint(&claims)?).await?.subject(),
            "user-123"
        );
        assert_eq!(
            verifier.verify(&server.mint(&claims)?).await?.subject(),
            "user-123"
        );
        assert_eq!(server.jwks_request_count(), 1);

        server.rotate_signing_key();
        assert_eq!(
            verifier.verify(&server.mint(&claims)?).await?.subject(),
            "user-123"
        );
        assert_eq!(server.jwks_request_count(), 2);
        Ok(())
    }

    #[tokio::test]
    async fn mock_server_exercises_jwks_timeout() -> Result<(), Box<dyn std::error::Error>> {
        let server = MockOidcServer::start().await?;
        // The timeout covers discovery too, and discovery is served without delay,
        // so too tight a budget fails discovery instead of the JWKS fetch under test.
        // Measured discovery is under 30 ms, but the margin has to absorb a clock
        // jump, not just slow I/O, so keep it wide and keep the delay well past it.
        server.set_jwks_delay(Duration::from_secs(10));
        let config = OidcConfig::new(server.issuer(), "api")?
            .with_request_timeout(Duration::from_secs(1))?;
        let verifier = OidcVerifier::discover(config).await?;
        let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
        assert!(matches!(
            verifier.verify(&server.mint(&claims)?).await,
            Err(VerificationError::JwksTimeout)
        ));
        Ok(())
    }

    #[tokio::test]
    async fn unknown_key_refresh_is_single_flight_and_negatively_cached()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = MockOidcServer::start().await?;
        server.set_jwks_delay(Duration::from_millis(20));
        let verifier =
            Arc::new(OidcVerifier::discover(OidcConfig::new(server.issuer(), "api")?).await?);
        let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
        let token = rs256_token_with_key_id(KEY_1_PEM, Some("unknown-key"), &claims)?;
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let verifier = Arc::clone(&verifier);
            let token = token.clone();
            tasks.push(tokio::spawn(async move { verifier.verify(&token).await }));
        }
        for task in tasks {
            assert!(matches!(task.await?, Err(VerificationError::UnknownKeyId)));
        }
        assert_eq!(server.jwks_request_count(), 1);

        assert!(matches!(
            verifier.verify(&token).await,
            Err(VerificationError::UnknownKeyId)
        ));
        assert_eq!(server.jwks_request_count(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn mock_sessions_cover_short_lived_concurrent_rejected_and_revoked_refresh()
    -> Result<(), Box<dyn std::error::Error>> {
        let server = Arc::new(MockOidcServer::start().await?);
        let verifier = OidcVerifier::discover(
            OidcConfig::new(server.issuer(), "api")?.with_clock_skew(Duration::ZERO),
        )
        .await?;
        let session = server.issue_session("user-123", "api", Duration::from_secs(1))?;
        assert_eq!(session.expires_in(), Duration::from_secs(1));
        assert_eq!(
            verifier.verify(session.access_token()).await?.subject(),
            "user-123"
        );

        server.set_refresh_delay(Duration::from_millis(20));
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let server = Arc::clone(&server);
            let refresh_token = session.refresh_token().to_owned();
            tasks.push(tokio::spawn(async move {
                server.refresh_session(&refresh_token).await
            }));
        }
        for task in tasks {
            let refreshed = task.await??;
            assert_eq!(
                verifier.verify(refreshed.access_token()).await?.subject(),
                "user-123"
            );
        }
        assert_eq!(server.refresh_request_count(), 8);

        assert!(server.reject_refresh(session.refresh_token()));
        assert!(matches!(
            server.refresh_session(session.refresh_token()).await,
            Err(JwtFixtureError::RefreshRejected { ref code }) if code == "invalid_grant"
        ));

        let revoked = server.issue_session("user-456", "api", Duration::from_secs(60))?;
        assert!(server.revoke_session(revoked.refresh_token()));
        assert!(matches!(
            server.refresh_session(revoked.refresh_token()).await,
            Err(JwtFixtureError::RefreshRejected { ref code }) if code == "invalid_grant"
        ));
        Ok(())
    }
}
