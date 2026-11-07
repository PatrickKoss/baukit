use std::{
    collections::BTreeMap,
    error::Error,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{CACHE_CONTROL, CONTENT_TYPE, COOKIE, ORIGIN, RETRY_AFTER, SET_COOKIE},
    },
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_credential_vault::CredentialCipher;
use baukit_egress::EgressError;
use baukit_http::HttpOptions;
use chrono::{DateTime, TimeDelta, TimeZone as _, Utc};
use serde_json::Value;
use tokio::sync::{Mutex, OwnedMutexGuard};
use tower::ServiceExt as _;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BROWSER_SESSION_HEADER, BrowserSessionConfig,
    BrowserSessionConfigError, BrowserSessionRateLimit, BrowserSessionRateLimiter,
    BrowserSessionState, RateLimitFuture, RateLimitOutcome, RateLimitSubject,
    config::{is_same_site, parse_origin},
    cookie::CookieSettings,
    crypto::{self, GRACE_ACCESS_TOKEN_FIELD, REFRESH_TOKEN_FIELD, SessionCredential},
    idp::{IdTokenVerifier, IdentityProvider, IdpError, IdpFuture, TokenSet},
    router,
    service::Clock,
    store::{SessionRow, SessionStore, SessionTransaction, StoreFuture},
};

type TestError = Box<dyn Error + Send + Sync>;
type TestResult = Result<(), TestError>;

const WEB_ORIGIN: &str = "http://localhost:5173";
const API_ORIGIN: &str = "http://localhost:8080";
const REDIRECT_URI: &str = "http://localhost:5173/auth/callback";
const SUBJECT: &str = "alice";
const ACCESS_LIFETIME: Duration = Duration::from_secs(300);

#[derive(Default)]
struct MemoryStore {
    rows: Arc<Mutex<BTreeMap<Uuid, SessionRow>>>,
}

impl MemoryStore {
    async fn snapshot(&self) -> BTreeMap<Uuid, SessionRow> {
        self.rows.lock().await.clone()
    }
}

struct MemoryTransaction {
    guard: OwnedMutexGuard<BTreeMap<Uuid, SessionRow>>,
    working: BTreeMap<Uuid, SessionRow>,
}

impl SessionStore for MemoryStore {
    fn begin(&self) -> StoreFuture<'_, Box<dyn SessionTransaction>> {
        Box::pin(async move {
            let guard = Arc::clone(&self.rows).lock_owned().await;
            let working = guard.clone();
            Ok(Box::new(MemoryTransaction { guard, working }) as Box<dyn SessionTransaction>)
        })
    }
}

fn clear_grace(row: &mut SessionRow) {
    row.grace_access_token = None;
    row.grace_expires_at = None;
    row.grace_until = None;
}

impl SessionTransaction for MemoryTransaction {
    fn lock<'a>(&'a mut self, id: Uuid) -> StoreFuture<'a, Option<SessionRow>> {
        Box::pin(std::future::ready(Ok(self.working.get(&id).cloned())))
    }

    fn insert<'a>(&'a mut self, row: &'a SessionRow) -> StoreFuture<'a, ()> {
        self.working.insert(row.id, row.clone());
        Box::pin(std::future::ready(Ok(())))
    }

    fn mark_rotated<'a>(
        &'a mut self,
        id: Uuid,
        successor: Uuid,
        now: DateTime<Utc>,
    ) -> StoreFuture<'a, ()> {
        if let Some(row) = self.working.get_mut(&id) {
            row.rotated_to = Some(successor);
            row.last_used_at = now;
            clear_grace(row);
        }
        Box::pin(std::future::ready(Ok(())))
    }

    fn revoke_family<'a>(&'a mut self, family_id: Uuid, now: DateTime<Utc>) -> StoreFuture<'a, ()> {
        for row in self.working.values_mut() {
            if row.family_id == family_id && row.revoked_at.is_none() {
                row.revoked_at = Some(now);
                clear_grace(row);
            }
        }
        Box::pin(std::future::ready(Ok(())))
    }

    fn delete_family<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, ()> {
        self.working.retain(|_, row| row.family_id != family_id);
        Box::pin(std::future::ready(Ok(())))
    }

    fn family_head<'a>(&'a mut self, family_id: Uuid) -> StoreFuture<'a, Option<SessionRow>> {
        let head = self
            .working
            .values()
            .filter(|row| row.family_id == family_id && row.rotated_to.is_none())
            .max_by_key(|row| row.created_at)
            .cloned();
        Box::pin(std::future::ready(Ok(head)))
    }

    fn commit(mut self: Box<Self>) -> StoreFuture<'static, ()> {
        *self.guard = std::mem::take(&mut self.working);
        Box::pin(std::future::ready(Ok(())))
    }
}

#[derive(Default)]
struct FakeIdp {
    refreshes: AtomicUsize,
    revoked: StdMutex<Vec<String>>,
    refresh_delay: Option<Duration>,
    unavailable: bool,
    id_tokens: Option<IdTokenVerifier>,
    exchange_id_token: Option<String>,
}

impl FakeIdp {
    fn tokens(&self, serial: usize) -> TokenSet {
        let id_token = self
            .exchange_id_token
            .clone()
            .unwrap_or_else(|| format!("id:{SUBJECT}"));
        TokenSet {
            access_token: Zeroizing::new(format!("access-{serial}")),
            expires_in: ACCESS_LIFETIME,
            refresh_token: Some(Zeroizing::new(format!("refresh-{serial}"))),
            refresh_expires_in: None,
            id_token: Some(Zeroizing::new(id_token)),
        }
    }

    fn revoked(&self) -> Vec<String> {
        self.revoked
            .lock()
            .map(|revoked| revoked.clone())
            .unwrap_or_default()
    }

    fn refresh_count(&self) -> usize {
        self.refreshes.load(Ordering::SeqCst)
    }
}

impl IdentityProvider for FakeIdp {
    fn exchange<'a>(
        &'a self,
        code: &'a str,
        _code_verifier: &'a str,
        _redirect_uri: &'a str,
    ) -> IdpFuture<'a, TokenSet> {
        let result = if code == "good-code" {
            Ok(self.tokens(0))
        } else {
            Err(IdpError::InvalidGrant)
        };
        Box::pin(std::future::ready(result))
    }

    fn refresh<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, TokenSet> {
        Box::pin(async move {
            if let Some(delay) = self.refresh_delay {
                tokio::time::sleep(delay).await;
            }
            if self.unavailable {
                return Err(IdpError::Unavailable(EgressError::Timeout));
            }
            if self
                .revoked()
                .iter()
                .any(|revoked| revoked == refresh_token)
            {
                return Err(IdpError::InvalidGrant);
            }
            let serial = self.refreshes.fetch_add(1, Ordering::SeqCst) + 1;
            Ok(self.tokens(serial))
        })
    }

    fn revoke<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, ()> {
        if let Ok(mut revoked) = self.revoked.lock() {
            revoked.push(refresh_token.to_owned());
        }
        Box::pin(std::future::ready(Ok(())))
    }

    fn subject<'a>(&'a self, id_token: &'a str) -> IdpFuture<'a, String> {
        Box::pin(async move {
            if let Some(verifier) = &self.id_tokens {
                return verifier.subject(id_token).await;
            }
            id_token
                .strip_prefix("id:")
                .map(str::to_owned)
                .ok_or(IdpError::InvalidResponse("bad id token"))
        })
    }

    fn end_session_url(&self, id_token: Option<&str>) -> Option<String> {
        Some(format!(
            "https://idp.example/logout?hint={}",
            id_token.unwrap_or("none")
        ))
    }
}

struct FakeClock {
    now: StdMutex<DateTime<Utc>>,
}

impl FakeClock {
    fn new() -> Self {
        let start = Utc
            .with_ymd_and_hms(2026, 11, 7, 12, 0, 0)
            .single()
            .unwrap_or(DateTime::<Utc>::MIN_UTC);
        Self {
            now: StdMutex::new(start),
        }
    }

    fn advance(&self, delta: TimeDelta) {
        if let Ok(mut now) = self.now.lock() {
            *now += delta;
        }
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        self.now
            .lock()
            .map(|now| *now)
            .unwrap_or(DateTime::<Utc>::MIN_UTC)
    }
}

#[derive(Default)]
struct RecordingLimiter {
    calls: StdMutex<Vec<&'static str>>,
    limited_group: Option<&'static str>,
}

impl RecordingLimiter {
    fn calls(&self) -> Vec<&'static str> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

impl BrowserSessionRateLimiter for RecordingLimiter {
    fn check<'a>(
        &'a self,
        limit: BrowserSessionRateLimit,
        _subject: RateLimitSubject<'a>,
    ) -> RateLimitFuture<'a> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(limit.group);
        }
        let outcome = if self.limited_group == Some(limit.group) {
            RateLimitOutcome::Limited {
                retry_after: Duration::from_secs(7),
            }
        } else {
            RateLimitOutcome::Allowed
        };
        Box::pin(std::future::ready(outcome))
    }
}

fn cipher() -> Result<CredentialCipher, TestError> {
    Ok(CredentialCipher::parse(&format!(
        "1:{}",
        STANDARD.encode([9_u8; 32])
    ))?)
}

fn local_config() -> Result<BrowserSessionConfig, TestError> {
    Ok(BrowserSessionConfig::new(
        "http://localhost:8180/realms/test",
        "web",
        "acme-session",
        API_ORIGIN,
        cipher()?,
    )?
    .with_redirect_uris([REDIRECT_URI]))
}

fn production_config() -> Result<BrowserSessionConfig, TestError> {
    Ok(BrowserSessionConfig::new(
        "https://identity.example.com/realms/test",
        "web",
        "acme-session",
        "https://api.example.com",
        cipher()?,
    )?
    .with_redirect_uris(["https://app.example.com/auth/callback"]))
}

fn http_options(origins: &[&str]) -> Result<HttpOptions, TestError> {
    let mut options = HttpOptions::default().with_allowed_origins(origins.iter().copied())?;
    options.allow_credentials = true;
    Ok(options)
}

struct Harness {
    app: Router,
    store: Arc<MemoryStore>,
    idp: Arc<FakeIdp>,
    clock: Arc<FakeClock>,
    limiter: Arc<RecordingLimiter>,
}

fn harness_with(
    config: BrowserSessionConfig,
    origins: &[&str],
    idp: FakeIdp,
    limiter: RecordingLimiter,
) -> Result<Harness, TestError> {
    let store = Arc::new(MemoryStore::default());
    let idp = Arc::new(idp);
    let clock = Arc::new(FakeClock::new());
    let limiter = Arc::new(limiter);
    let state = BrowserSessionState::from_parts(
        config,
        &http_options(origins)?,
        Arc::clone(&store) as Arc<dyn SessionStore>,
        Arc::clone(&idp) as Arc<dyn IdentityProvider>,
        Arc::clone(&clock) as Arc<dyn Clock>,
        Arc::clone(&limiter) as Arc<dyn BrowserSessionRateLimiter>,
    )?;
    Ok(Harness {
        app: router(state),
        store,
        idp,
        clock,
        limiter,
    })
}

fn harness(idp: FakeIdp) -> Result<Harness, TestError> {
    harness_with(
        local_config()?.with_insecure_local_cookie(true),
        &[WEB_ORIGIN],
        idp,
        RecordingLimiter::default(),
    )
}

fn request(
    path: &str,
    cookie: Option<&str>,
    body: Option<&Value>,
) -> Result<Request<Body>, TestError> {
    let mut builder = Request::post(format!("/auth/session/{path}"))
        .header(ORIGIN, WEB_ORIGIN)
        .header(BROWSER_SESSION_HEADER, "1");
    if let Some(cookie) = cookie {
        builder = builder.header(COOKIE, cookie);
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(body.to_string())
        }
        None => Body::empty(),
    };
    Ok(builder.body(body)?)
}

fn exchange_body() -> Value {
    serde_json::json!({
        "code": "good-code",
        "codeVerifier": "verifier",
        "redirectUri": REDIRECT_URI,
    })
}

fn set_cookie(response: &Response) -> Option<String> {
    response
        .headers()
        .get(SET_COOKIE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn cookie_pair(set_cookie: &str) -> Result<String, TestError> {
    Ok(set_cookie
        .split(';')
        .next()
        .ok_or("empty Set-Cookie")?
        .to_owned())
}

async fn json(response: Response) -> Result<Value, TestError> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn send(app: &Router, request: Request<Body>) -> Result<Response, TestError> {
    Ok(app.clone().oneshot(request).await?)
}

async fn signed_in(harness: &Harness) -> Result<String, TestError> {
    let response = send(
        &harness.app,
        request("exchange", None, Some(&exchange_body()))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    cookie_pair(&set_cookie(&response).ok_or("exchange set no cookie")?)
}

struct Refreshed {
    status: StatusCode,
    set_cookie: Option<String>,
    body: Value,
}

async fn refreshed(harness: &Harness, cookie: &str) -> Result<Refreshed, TestError> {
    let response = send(&harness.app, request("refresh", Some(cookie), None)?).await?;
    let status = response.status();
    let set_cookie = set_cookie(&response);
    Ok(Refreshed {
        status,
        set_cookie,
        body: json(response).await?,
    })
}

async fn rotated_cookie(harness: &Harness, cookie: &str) -> Result<String, TestError> {
    let refreshed = refreshed(harness, cookie).await?;
    assert_eq!(refreshed.status, StatusCode::OK);
    cookie_pair(&refreshed.set_cookie.ok_or("no rotated cookie")?)
}

#[test]
fn production_cookie_has_host_prefix_and_secure_attributes() -> TestResult {
    let config = production_config()?;
    let cookie = CookieSettings::new(config.cookie_name(), true);
    let credential = SessionCredential::generate(&aws_lc_rs::rand::SystemRandom::new())?;
    let header = cookie
        .set(&credential, Duration::from_secs(2_592_000))
        .ok_or("cookie header")?;
    let header = header.to_str()?;
    assert!(header.starts_with("__Host-acme-session="));
    assert!(header.ends_with("; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=2592000"));
    assert!(!header.to_ascii_lowercase().contains("domain"));
    let cleared = cookie.clear().ok_or("clear header")?;
    assert_eq!(
        cleared.to_str()?,
        "__Host-acme-session=; Path=/; Secure; HttpOnly; SameSite=Strict; Max-Age=0"
    );
    Ok(())
}

#[tokio::test]
async fn insecure_local_cookie_drops_prefix_and_secure() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let response = send(
        &harness.app,
        request("exchange", None, Some(&exchange_body()))?,
    )
    .await?;
    let cookie = set_cookie(&response).ok_or("no cookie")?;
    assert!(cookie.starts_with("acme-session="));
    assert!(cookie.ends_with("; Path=/; HttpOnly; SameSite=Strict; Max-Age=2592000"));
    Ok(())
}

#[test]
fn insecure_local_cookie_is_refused_with_a_non_local_origin() -> TestResult {
    let config = production_config()?.with_insecure_local_cookie(true);
    let result = config.validate(&http_options(&["https://app.example.com"])?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::InsecureCookieNotLocal)
    ));
    let mixed = local_config()?.with_insecure_local_cookie(true);
    let result = mixed.validate(&http_options(&[WEB_ORIGIN, "http://app.localtest.me"])?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::InsecureCookieNotLocal)
    ));
    let local = local_config()?.with_insecure_local_cookie(true);
    assert!(local.validate(&http_options(&[WEB_ORIGIN])?).is_ok());
    Ok(())
}

#[test]
fn router_refuses_empty_origins_and_disabled_credentials() -> TestResult {
    let config = local_config()?;
    let result = config.validate(&http_options(&[])?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::NoAllowedOrigins)
    ));
    let mut without_credentials = http_options(&[WEB_ORIGIN])?;
    without_credentials.allow_credentials = false;
    let result = config.validate(&without_credentials);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::CredentialsDisabled)
    ));
    let without_redirects = local_config()?.with_redirect_uris(Vec::<String>::new());
    let result = without_redirects.validate(&http_options(&[WEB_ORIGIN])?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::NoRedirectUris)
    ));
    let result = harness_with(
        local_config()?,
        &[],
        FakeIdp::default(),
        RecordingLimiter::default(),
    );
    assert!(result.is_err());
    Ok(())
}

fn same_site(left: &str, right: &str) -> Result<bool, TestError> {
    let left = parse_origin(left).ok_or("left origin")?;
    let right = parse_origin(right).ok_or("right origin")?;
    Ok(is_same_site(&left, &right))
}

#[test]
fn same_site_check_uses_the_public_suffix_list() -> TestResult {
    assert!(same_site(
        "https://app.example.com",
        "https://api.example.com"
    )?);
    assert!(same_site("https://example.com", "https://api.example.com")?);
    assert!(same_site(
        "https://app.example.co.uk",
        "https://api.example.co.uk"
    )?);
    assert!(same_site("http://localhost:5173", "http://localhost:8080")?);
    assert!(!same_site("https://a.github.io", "https://b.github.io")?);
    assert!(!same_site("https://example.co.uk", "https://other.co.uk")?);
    assert!(!same_site(
        "https://app.example.com",
        "https://api.example.org"
    )?);
    assert!(!same_site(
        "http://app.example.com",
        "https://api.example.com"
    )?);
    assert!(!same_site(
        "http://127.0.0.1:5173",
        "http://localhost:8080"
    )?);
    let config = production_config()?;
    assert!(
        config
            .validate(&http_options(&["https://app.example.com"])?)
            .is_ok()
    );
    let result = config.validate(&http_options(&[
        "https://app.example.com",
        "https://app.example.net",
    ])?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::CrossSiteOrigin(origin)) if origin == "https://app.example.net"
    ));
    Ok(())
}

#[test]
fn parse_origin_normalizes_and_rejects_non_http() {
    assert_eq!(
        parse_origin("https://App.Example.com:443/path").map(String::from),
        Some("https://app.example.com/".to_owned())
    );
    assert!(parse_origin("file:///etc/passwd").is_none());
    assert!(parse_origin("null").is_none());
}

fn csrf_cases() -> Vec<(&'static str, Option<&'static str>, Option<&'static str>)> {
    vec![
        ("missing header", Some(WEB_ORIGIN), None),
        ("wrong header value", Some(WEB_ORIGIN), Some("true")),
        ("missing origin", None, Some("1")),
        ("foreign origin", Some("https://evil.example"), Some("1")),
        (
            "same-site foreign port",
            Some("http://localhost:9999"),
            Some("1"),
        ),
        ("opaque origin", Some("null"), Some("1")),
    ]
}

#[tokio::test]
async fn csrf_matrix_rejects_every_incomplete_request() -> TestResult {
    let harness = harness_with(
        local_config()?.with_insecure_local_cookie(true),
        &[WEB_ORIGIN, "http://localhost:9000"],
        FakeIdp::default(),
        RecordingLimiter::default(),
    )?;
    for (name, origin, marker) in csrf_cases() {
        for path in ["exchange", "refresh", "logout"] {
            let mut builder = Request::post(format!("/auth/session/{path}"));
            if let Some(origin) = origin {
                builder = builder.header(ORIGIN, origin);
            }
            if let Some(marker) = marker {
                builder = builder.header(BROWSER_SESSION_HEADER, marker);
            }
            let response = send(&harness.app, builder.body(Body::empty())?).await?;
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name} on {path}");
            assert_eq!(
                response
                    .headers()
                    .get(CACHE_CONTROL)
                    .map(|value| value.as_bytes()),
                Some(b"no-store".as_slice())
            );
            assert_eq!(json(response).await?["error"]["code"], "csrf_rejected");
        }
    }
    assert!(harness.limiter.calls().is_empty());
    let allowed = send(&harness.app, request("refresh", None, None)?).await?;
    assert_eq!(allowed.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}

#[tokio::test]
async fn exchange_rejects_an_unknown_redirect_uri_and_a_bad_code() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let mut body = exchange_body();
    body["redirectUri"] = Value::from("https://evil.example/callback");
    let response = send(&harness.app, request("exchange", None, Some(&body))?).await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json(response).await?["error"]["code"], "invalid_grant");
    let mut body = exchange_body();
    body["code"] = Value::from("used-code");
    let response = send(&harness.app, request("exchange", None, Some(&body))?).await?;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(harness.store.snapshot().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn exchange_returns_the_access_token_and_stores_only_ciphertext() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let response = send(
        &harness.app,
        request("exchange", None, Some(&exchange_body()))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(CACHE_CONTROL)
            .map(|value| value.as_bytes()),
        Some(b"no-store".as_slice())
    );
    let body = json(response).await?;
    assert_eq!(body["accessToken"], "access-0");
    assert_eq!(body["subject"], SUBJECT);
    let expected = harness.clock.now() + TimeDelta::seconds(300);
    assert_eq!(body["expiresAt"], expected.timestamp_millis());
    let rows = harness.store.snapshot().await;
    let row = rows.values().next().ok_or("no row")?;
    assert!(
        !row.refresh_token
            .windows(9)
            .any(|window| window == b"refresh-0")
    );
    assert_eq!(row.secret_hash.len(), 32);
    assert_eq!(row.family_id, row.id);
    Ok(())
}

#[tokio::test]
async fn refresh_rotates_the_cookie() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let refreshed_once = refreshed(&harness, &first).await?;
    assert_eq!(refreshed_once.status, StatusCode::OK);
    assert_eq!(refreshed_once.body["accessToken"], "access-1");
    let second = cookie_pair(&refreshed_once.set_cookie.ok_or("no rotated cookie")?)?;
    assert_ne!(second, first);
    let refreshed_twice = refreshed(&harness, &second).await?;
    assert_eq!(refreshed_twice.status, StatusCode::OK);
    assert_eq!(refreshed_twice.body["accessToken"], "access-2");
    Ok(())
}

#[tokio::test]
async fn old_secret_inside_the_grace_window_gets_the_successor_token() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let rotated = refreshed(&harness, &first).await?;
    harness.clock.advance(TimeDelta::seconds(10));
    let replay = refreshed(&harness, &first).await?;
    assert_eq!(replay.status, StatusCode::OK);
    assert!(replay.set_cookie.is_none());
    assert_eq!(replay.body["accessToken"], rotated.body["accessToken"]);
    assert_eq!(replay.body["expiresAt"], rotated.body["expiresAt"]);
    assert_eq!(harness.idp.refresh_count(), 1);
    assert_eq!(harness.store.snapshot().await.len(), 2);
    Ok(())
}

#[tokio::test]
async fn old_secret_after_the_grace_window_revokes_the_family() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let second = rotated_cookie(&harness, &first).await?;
    harness
        .clock
        .advance(TimeDelta::seconds(10) + TimeDelta::milliseconds(1));
    let replay = refreshed(&harness, &first).await?;
    assert_eq!(replay.status, StatusCode::UNAUTHORIZED);
    assert_eq!(replay.body["error"]["code"], "session_expired");
    assert!(
        replay
            .set_cookie
            .ok_or("no cleared cookie")?
            .contains("Max-Age=0")
    );
    assert_eq!(harness.idp.revoked(), ["refresh-1"]);
    assert!(
        harness
            .store
            .snapshot()
            .await
            .values()
            .all(|row| row.revoked_at.is_some())
    );
    assert_eq!(
        refreshed(&harness, &second).await?.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(harness.idp.refresh_count(), 1);
    Ok(())
}

#[tokio::test]
async fn concurrent_refreshes_make_one_idp_call_and_one_successor() -> TestResult {
    let idp = FakeIdp {
        refresh_delay: Some(Duration::from_millis(50)),
        ..FakeIdp::default()
    };
    let harness = harness(idp)?;
    let first = signed_in(&harness).await?;
    let (left, right) = tokio::join!(refreshed(&harness, &first), refreshed(&harness, &first));
    let (left, right) = (left?, right?);
    assert_eq!(left.status, StatusCode::OK);
    assert_eq!(right.status, StatusCode::OK);
    assert_eq!(left.body["accessToken"], right.body["accessToken"]);
    assert_eq!(
        usize::from(left.set_cookie.is_some()) + usize::from(right.set_cookie.is_some()),
        1
    );
    assert_eq!(harness.idp.refresh_count(), 1);
    assert_eq!(harness.store.snapshot().await.len(), 2);
    Ok(())
}

#[tokio::test]
async fn rotating_the_successor_clears_its_grace_token() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let second = rotated_cookie(&harness, &first).await?;
    rotated_cookie(&harness, &second).await?;
    let rows = harness.store.snapshot().await;
    let rotated = rows
        .values()
        .filter(|row| row.rotated_to.is_some())
        .collect::<Vec<_>>();
    assert_eq!(rotated.len(), 2);
    assert!(rotated.iter().all(|row| row.grace_access_token.is_none()));
    assert_eq!(
        refreshed(&harness, &first).await?.status,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn grace_token_sealed_for_one_row_fails_under_another_aad() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    rotated_cookie(&harness, &first).await?;
    let rows = harness.store.snapshot().await;
    let successor = rows
        .values()
        .find(|row| row.grace_access_token.is_some())
        .ok_or("no successor")?;
    let original = rows
        .values()
        .find(|row| row.rotated_to.is_some())
        .ok_or("no original")?;
    let sealed = successor
        .grace_access_token
        .as_ref()
        .ok_or("no grace token")?;
    let cipher = cipher()?;
    let opened = crypto::open(
        &cipher,
        successor.id,
        &successor.subject,
        successor.key_version,
        GRACE_ACCESS_TOKEN_FIELD,
        sealed,
    )?;
    assert_eq!(opened.as_str(), "access-1");
    for (row_id, subject, field) in [
        (original.id, SUBJECT, GRACE_ACCESS_TOKEN_FIELD),
        (successor.id, "mallory", GRACE_ACCESS_TOKEN_FIELD),
        (successor.id, SUBJECT, REFRESH_TOKEN_FIELD),
    ] {
        let result = crypto::open(
            &cipher,
            row_id,
            subject,
            successor.key_version,
            field,
            sealed,
        );
        assert!(result.is_err());
    }
    Ok(())
}

#[tokio::test]
async fn logout_revokes_deletes_and_returns_the_end_session_url() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    let response = send(&harness.app, request("logout", Some(&first), None)?).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        set_cookie(&response)
            .ok_or("no cleared cookie")?
            .contains("Max-Age=0")
    );
    let body = json(response).await?;
    assert_eq!(
        body["endSessionUrl"],
        format!("https://idp.example/logout?hint=id:{SUBJECT}")
    );
    assert_eq!(harness.idp.revoked(), ["refresh-0"]);
    assert!(harness.store.snapshot().await.is_empty());
    assert_eq!(
        refreshed(&harness, &first).await?.status,
        StatusCode::UNAUTHORIZED
    );
    let response = send(&harness.app, request("logout", None, None)?).await?;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    Ok(())
}

#[tokio::test]
async fn refresh_without_a_valid_cookie_is_expired() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let refreshed = refreshed(&harness, "acme-session=garbage").await?;
    assert_eq!(refreshed.status, StatusCode::UNAUTHORIZED);
    assert_eq!(refreshed.body["error"]["code"], "session_expired");
    assert!(refreshed.set_cookie.is_some());
    Ok(())
}

#[tokio::test]
async fn idp_outage_is_retryable_and_keeps_the_session() -> TestResult {
    let harness = harness(FakeIdp {
        unavailable: true,
        ..FakeIdp::default()
    })?;
    let first = signed_in(&harness).await?;
    let refreshed = refreshed(&harness, &first).await?;
    assert_eq!(refreshed.status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(refreshed.body["error"]["code"], "idp_unavailable");
    assert!(refreshed.set_cookie.is_none());
    let rows = harness.store.snapshot().await;
    assert!(
        rows.values()
            .all(|row| row.revoked_at.is_none() && row.rotated_to.is_none())
    );
    Ok(())
}

#[tokio::test]
async fn idle_timeout_ends_the_session_without_an_idp_call() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    harness.clock.advance(TimeDelta::days(30));
    assert_eq!(
        refreshed(&harness, &first).await?.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(harness.idp.refresh_count(), 0);
    Ok(())
}

#[tokio::test]
async fn rate_limits_use_the_ip_and_session_buckets() -> TestResult {
    let harness = harness(FakeIdp::default())?;
    let first = signed_in(&harness).await?;
    refreshed(&harness, &first).await?;
    assert_eq!(
        harness.limiter.calls(),
        [
            AUTH_SESSION_IP_LIMIT.group,
            AUTH_SESSION_IP_LIMIT.group,
            AUTH_SESSION_ID_LIMIT.group
        ]
    );
    let limited = harness_with(
        local_config()?.with_insecure_local_cookie(true),
        &[WEB_ORIGIN],
        FakeIdp::default(),
        RecordingLimiter {
            limited_group: Some(AUTH_SESSION_IP_LIMIT.group),
            ..RecordingLimiter::default()
        },
    )?;
    let response = send(
        &limited.app,
        request("exchange", None, Some(&exchange_body()))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response
            .headers()
            .get(RETRY_AFTER)
            .map(|value| value.as_bytes()),
        Some(b"7".as_slice())
    );
    assert_eq!(json(response).await?["error"]["code"], "rate_limited");
    Ok(())
}

const CLIENT_ID: &str = "web";
const ID_TOKEN_LIFETIME: Duration = Duration::from_secs(300);
const EXPIRED_SECONDS_AGO: u64 = 3_600;

async fn exchange_with_id_token(
    server: &baukit_test::MockOidcServer,
    id_token: String,
) -> Result<(Response, Harness), TestError> {
    let verifier = IdTokenVerifier::new(
        server.issuer(),
        CLIENT_ID,
        server.jwks_url(),
        Duration::from_secs(5),
    )?;
    let harness = harness(FakeIdp {
        id_tokens: Some(verifier),
        exchange_id_token: Some(id_token),
        ..FakeIdp::default()
    })?;
    let response = send(
        &harness.app,
        request("exchange", None, Some(&exchange_body()))?,
    )
    .await?;
    Ok((response, harness))
}

#[tokio::test]
async fn exchange_accepts_a_verified_id_token() -> TestResult {
    let server = baukit_test::MockOidcServer::start().await?;
    let token = server.mint(&server.claims(SUBJECT, CLIENT_ID, ID_TOKEN_LIFETIME)?)?;
    let (response, harness) = exchange_with_id_token(&server, token).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json(response).await?["subject"], SUBJECT);
    assert_eq!(harness.store.snapshot().await.len(), 1);
    Ok(())
}

#[tokio::test]
async fn exchange_rejects_unverifiable_id_tokens_without_a_session() -> TestResult {
    let server = baukit_test::MockOidcServer::start().await?;
    let valid = server.claims(SUBJECT, CLIENT_ID, ID_TOKEN_LIFETIME)?;
    let forged = forge_signature(&server.mint(&valid)?)?;
    let wrong_audience =
        server.mint(&server.claims(SUBJECT, "other-client", ID_TOKEN_LIFETIME)?)?;
    let past = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs()
        .saturating_sub(EXPIRED_SECONDS_AGO);
    let expired = server.mint(&valid.expires_at(past))?;
    for (case, token) in [
        ("forged", forged),
        ("wrong audience", wrong_audience),
        ("expired", expired),
    ] {
        let (response, harness) = exchange_with_id_token(&server, token).await?;
        assert_eq!(
            response.status(),
            StatusCode::INTERNAL_SERVER_ERROR,
            "{case}"
        );
        assert!(set_cookie(&response).is_none(), "{case}");
        assert!(harness.store.snapshot().await.is_empty(), "{case}");
    }
    Ok(())
}

fn forge_signature(token: &str) -> Result<String, TestError> {
    let (signed, signature) = token.rsplit_once('.').ok_or("token has no signature")?;
    let mut bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(signature)?;
    let first = bytes.first_mut().ok_or("empty signature")?;
    *first ^= 0xff;
    Ok(format!(
        "{signed}.{}",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    ))
}
