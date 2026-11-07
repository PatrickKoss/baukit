//! Browser session BFF against real Keycloak, PostgreSQL and Redis.
#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    reason = "integration test code: a failed expectation is the test failure, and scenario tests read top to bottom"
)]

use std::{
    error::Error,
    future::Future,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use aws_lc_rs::digest;
use axum::{
    Router,
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{
        Request, StatusCode,
        header::{CONTENT_TYPE, COOKIE, LOCATION, ORIGIN, RETRY_AFTER, SET_COOKIE},
    },
    response::Response,
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
};
use baukit_auth::{
    OidcConfig, OidcVerifier,
    browser_session::{
        self, BROWSER_SESSION_HEADER, BrowserSessionConfig, BrowserSessionRateLimit,
        BrowserSessionRateLimiter, BrowserSessionState, RateLimitFuture, RateLimitOutcome,
    },
};
use baukit_credential_vault::{
    CredentialCipher, EncryptedCredentials, EncryptedField, NONCE_LENGTH,
};
use baukit_http::HttpOptions;
use baukit_ratelimit::{RateLimitOptions, RedisRateLimitStore, StoreBrowserSessionRateLimiter};
use baukit_test::{KeycloakTestContainer, PostgresTestContainer, RedisTestContainer};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt as _;
use url::{Url, form_urlencoded};
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;
type TestResult = Result<(), TestError>;

const REALM: &str = "baukit";
const CLIENT_ID: &str = "web";
const WEB_ORIGIN: &str = "http://localhost:5173";
const API_ORIGIN: &str = "http://localhost:8080";
const REDIRECT_URI: &str = "http://localhost:5173/auth/callback";
const USERNAME: &str = "alice";
const PASSWORD: &str = "alice-password";
const ACCESS_TOKEN_SECONDS: u64 = 10;
const KEY: [u8; 32] = [3; 32];
const AAD_DOMAIN: &[u8] = b"baukit-auth/browser-session/v1";

fn realm_json() -> Vec<u8> {
    serde_json::json!({
        "realm": REALM,
        "enabled": true,
        "sslRequired": "none",
        "accessTokenLifespan": ACCESS_TOKEN_SECONDS,
        "revokeRefreshToken": true,
        "refreshTokenMaxReuse": 0,
        "clients": [{
            "clientId": CLIENT_ID,
            "enabled": true,
            "publicClient": true,
            "standardFlowEnabled": true,
            "directAccessGrantsEnabled": false,
            "redirectUris": [format!("{WEB_ORIGIN}/*")],
            "webOrigins": ["+"],
            "attributes": {
                "pkce.code.challenge.method": "S256",
                "post.logout.redirect.uris": format!("{WEB_ORIGIN}/*"),
            },
            "protocolMappers": [{
                "name": "web-audience",
                "protocol": "openid-connect",
                "protocolMapper": "oidc-audience-mapper",
                "config": {
                    "included.client.audience": CLIENT_ID,
                    "access.token.claim": "true",
                    "id.token.claim": "false",
                },
            }],
        }],
        "users": [{
            "username": USERNAME,
            "enabled": true,
            "email": "alice@example.com",
            "emailVerified": true,
            "firstName": "Alice",
            "lastName": "Example",
            "credentials": [{"type": "password", "value": PASSWORD, "temporary": false}],
        }],
    })
    .to_string()
    .into_bytes()
}

struct AllowAll;

impl BrowserSessionRateLimiter for AllowAll {
    fn check<'a>(
        &'a self,
        _limit: BrowserSessionRateLimit,
        _subject: browser_session::RateLimitSubject<'a>,
    ) -> RateLimitFuture<'a> {
        Box::pin(std::future::ready(RateLimitOutcome::Allowed))
    }
}

struct Fixture {
    keycloak: KeycloakTestContainer,
    _postgres: PostgresTestContainer,
    _redis: Option<RedisTestContainer>,
    pool: PgPool,
    app: Router,
}

async fn fixture(with_redis: bool) -> Result<Fixture, TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let (keycloak, postgres) = tokio::try_join!(
        async { Ok::<_, TestError>(KeycloakTestContainer::start(REALM, realm_json()).await?) },
        async {
            Ok::<_, TestError>(baukit_test::start_postgres_with_migrations(migrations).await?)
        },
    )?;
    let pool = PgPool::connect(postgres.connection_url()).await?;
    let (redis, limiter): (
        Option<RedisTestContainer>,
        Arc<dyn BrowserSessionRateLimiter>,
    ) = if with_redis {
        let redis = baukit_test::start_redis().await?;
        let store = RedisRateLimitStore::connect(redis.connection_url()).await?;
        (
            Some(redis),
            Arc::new(StoreBrowserSessionRateLimiter::new(
                store,
                RateLimitOptions::default(),
            )),
        )
    } else {
        (None, Arc::new(AllowAll))
    };
    let config = BrowserSessionConfig::new(
        &keycloak.issuer(),
        CLIENT_ID,
        "acme-session",
        API_ORIGIN,
        CredentialCipher::parse(&format!("1:{}", STANDARD.encode(KEY)))?,
    )?
    .with_redirect_uris([REDIRECT_URI])
    .with_post_logout_redirect_uri(format!("{WEB_ORIGIN}/"))
    .with_insecure_local_cookie(true);
    let mut http = HttpOptions::default().with_allowed_origins([WEB_ORIGIN])?;
    http.allow_credentials = true;
    let state = BrowserSessionState::discover(config, &http, pool.clone(), limiter).await?;
    Ok(Fixture {
        keycloak,
        _postgres: postgres,
        _redis: redis,
        pool,
        app: browser_session::router(state),
    })
}

struct Authorization {
    code: String,
    verifier: String,
}

fn form(fields: &[(&str, &str)]) -> String {
    let mut serializer = form_urlencoded::Serializer::new(String::new());
    for (name, value) in fields {
        serializer.append_pair(name, value);
    }
    serializer.finish()
}

fn login_form_action(page: &str) -> Result<String, TestError> {
    let start = page.find("id=\"kc-form-login\"").ok_or("login form")?;
    let form_start = page
        .get(..start)
        .and_then(|head| head.rfind("<form"))
        .ok_or("form tag")?;
    let form = page.get(form_start..).ok_or("form slice")?;
    let (_, rest) = form.split_once("action=\"").ok_or("form action")?;
    let (action, _) = rest.split_once('"').ok_or("form action end")?;
    Ok(action.replace("&amp;", "&"))
}

async fn authorize(keycloak: &KeycloakTestContainer) -> Result<Authorization, TestError> {
    let client = baukit_test::test_http_client()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .build()?;
    let verifier = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
    let challenge = URL_SAFE_NO_PAD.encode(digest::digest(&digest::SHA256, verifier.as_bytes()));
    let authorization = Url::parse_with_params(
        &format!("{}/protocol/openid-connect/auth", keycloak.issuer()),
        [
            ("client_id", CLIENT_ID),
            ("redirect_uri", REDIRECT_URI),
            ("response_type", "code"),
            ("scope", "openid"),
            ("state", "state-1"),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", "S256"),
        ],
    )?;
    let page = client.get(authorization).send().await?;
    let cookies = page
        .headers()
        .get_all(SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok()?.split(';').next())
        .collect::<Vec<_>>()
        .join("; ");
    let action = login_form_action(&page.text().await?)?;
    let callback = client
        .post(action)
        .header(COOKIE, cookies)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[
            ("username", USERNAME),
            ("password", PASSWORD),
            ("credentialId", ""),
        ]))
        .send()
        .await?;
    let location = callback
        .headers()
        .get(LOCATION)
        .ok_or("login did not redirect")?
        .to_str()?;
    let code = Url::parse(location)?
        .query_pairs()
        .find(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
        .ok_or("callback has no code")?;
    Ok(Authorization { code, verifier })
}

fn session_request(
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

async fn send(app: &Router, request: Request<Body>) -> Result<Response, TestError> {
    Ok(app.clone().oneshot(request).await?)
}

async fn json(response: Response) -> Result<Value, TestError> {
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn cookie_of(response: &Response) -> Result<String, TestError> {
    let header = response
        .headers()
        .get(SET_COOKIE)
        .ok_or("no Set-Cookie")?
        .to_str()?;
    Ok(header
        .split(';')
        .next()
        .ok_or("empty Set-Cookie")?
        .to_owned())
}

async fn exchange(fixture: &Fixture) -> Result<(String, Value), TestError> {
    let authorization = authorize(&fixture.keycloak).await?;
    let body = serde_json::json!({
        "code": authorization.code,
        "codeVerifier": authorization.verifier,
        "redirectUri": REDIRECT_URI,
    });
    let response = send(
        &fixture.app,
        session_request("exchange", None, Some(&body))?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = cookie_of(&response)?;
    Ok((cookie, json(response).await?))
}

async fn refresh(fixture: &Fixture, cookie: &str) -> Result<Response, TestError> {
    send(
        &fixture.app,
        session_request("refresh", Some(cookie), None)?,
    )
    .await
}

fn session_id(cookie: &str) -> Result<Uuid, TestError> {
    let (_, value) = cookie.split_once('=').ok_or("cookie pair")?;
    let (id, _) = value.split_once('.').ok_or("cookie value")?;
    Ok(Uuid::try_parse(id)?)
}

fn aad_scope(row_id: Uuid, subject: &str) -> Uuid {
    let mut context = digest::Context::new(&digest::SHA256);
    context.update(AAD_DOMAIN);
    context.update(row_id.as_bytes());
    context.update(subject.as_bytes());
    let mut scope = [0_u8; 16];
    for (target, source) in scope.iter_mut().zip(context.finish().as_ref()) {
        *target = *source;
    }
    Uuid::from_bytes(scope)
}

async fn stored_refresh_token(pool: &PgPool, row_id: Uuid) -> Result<String, TestError> {
    let (subject, key_version, sealed): (String, i32, Vec<u8>) = sqlx::query_as(
        "SELECT subject, key_version, refresh_token FROM baukit_auth_browser_sessions WHERE id = $1",
    )
    .bind(row_id)
    .fetch_one(pool)
    .await?;
    let (nonce, ciphertext) = sealed
        .split_at_checked(NONCE_LENGTH)
        .ok_or("sealed too short")?;
    let encrypted = EncryptedCredentials {
        scope_id: aad_scope(row_id, &subject),
        fields: [(
            "refresh_token".to_owned(),
            EncryptedField {
                ciphertext: ciphertext.to_vec(),
                nonce: nonce.to_vec(),
            },
        )]
        .into(),
        key_version,
    };
    let cipher = CredentialCipher::parse(&format!("1:{}", STANDARD.encode(KEY)))?;
    let secrets = cipher.decrypt(&encrypted)?;
    Ok(String::from_utf8(
        secrets.get("refresh_token").ok_or("no field")?.to_vec(),
    )?)
}

async fn keycloak_refresh_status(
    keycloak: &KeycloakTestContainer,
    refresh_token: &str,
) -> Result<reqwest::StatusCode, TestError> {
    let response = baukit_test::test_http_client()
        .build()?
        .post(format!(
            "{}/protocol/openid-connect/token",
            keycloak.issuer()
        ))
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(form(&[
            ("grant_type", "refresh_token"),
            ("client_id", CLIENT_ID),
            ("refresh_token", refresh_token),
        ]))
        .send()
        .await?;
    Ok(response.status())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn code_exchange_refresh_rotation_and_logout_against_keycloak() -> TestResult {
    let fixture = fixture(false).await?;
    let (first, issued) = exchange(&fixture).await?;
    assert!(
        issued["accessToken"]
            .as_str()
            .is_some_and(|token| token.starts_with("eyJ"))
    );
    assert!(
        issued["subject"]
            .as_str()
            .is_some_and(|subject| !subject.is_empty())
    );
    let original_refresh = stored_refresh_token(&fixture.pool, session_id(&first)?).await?;

    let response = refresh(&fixture, &first).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let second = cookie_of(&response)?;
    assert_eq!(json(response).await?["subject"], issued["subject"]);
    let rotated_refresh = stored_refresh_token(&fixture.pool, session_id(&second)?).await?;
    assert_ne!(rotated_refresh, original_refresh);
    assert_eq!(
        keycloak_refresh_status(&fixture.keycloak, &original_refresh).await?,
        reqwest::StatusCode::BAD_REQUEST
    );

    let response = send(
        &fixture.app,
        session_request("logout", Some(&second), None)?,
    )
    .await?;
    assert_eq!(response.status(), StatusCode::OK);
    let end_session = json(response).await?;
    let end_session = end_session["endSessionUrl"]
        .as_str()
        .ok_or("no endSessionUrl")?;
    assert!(end_session.starts_with(&format!(
        "{}/protocol/openid-connect/logout?",
        fixture.keycloak.issuer()
    )));
    assert!(end_session.contains("id_token_hint="));
    assert_eq!(
        keycloak_refresh_status(&fixture.keycloak, &rotated_refresh).await?,
        reqwest::StatusCode::BAD_REQUEST
    );
    let rows: i64 = sqlx::query_scalar("SELECT count(*) FROM baukit_auth_browser_sessions")
        .fetch_one(&fixture.pool)
        .await?;
    assert_eq!(rows, 0);
    assert_eq!(
        refresh(&fixture, &second).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn reusing_a_rotated_cookie_after_the_grace_window_revokes_the_session() -> TestResult {
    let fixture = fixture(false).await?;
    let (first, _) = exchange(&fixture).await?;
    let response = refresh(&fixture, &first).await?;
    assert_eq!(response.status(), StatusCode::OK);
    let second = cookie_of(&response)?;
    let live_refresh = stored_refresh_token(&fixture.pool, session_id(&second)?).await?;

    let grace = refresh(&fixture, &first).await?;
    assert_eq!(grace.status(), StatusCode::OK);
    assert!(grace.headers().get(SET_COOKIE).is_none());

    tokio::time::sleep(Duration::from_secs(11)).await;
    let reuse = refresh(&fixture, &first).await?;
    assert_eq!(reuse.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(json(reuse).await?["error"]["code"], "session_expired");
    assert_eq!(
        refresh(&fixture, &second).await?.status(),
        StatusCode::UNAUTHORIZED
    );
    let live: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM baukit_auth_browser_sessions WHERE revoked_at IS NULL",
    )
    .fetch_one(&fixture.pool)
    .await?;
    assert_eq!(live, 0);
    assert_eq!(
        keycloak_refresh_status(&fixture.keycloak, &live_refresh).await?,
        reqwest::StatusCode::BAD_REQUEST
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn issued_access_token_is_rejected_by_the_verifier_after_expiry() -> TestResult {
    let fixture = fixture(false).await?;
    let (_, issued) = exchange(&fixture).await?;
    let token = issued["accessToken"].as_str().ok_or("no access token")?;
    let verifier = OidcVerifier::discover(
        OidcConfig::new(fixture.keycloak.issuer(), CLIENT_ID)?.with_clock_skew(Duration::ZERO),
    )
    .await?;
    let principal = verifier.verify(token).await?;
    assert_eq!(Some(principal.subject()), issued["subject"].as_str());
    let expires_at = issued["expiresAt"].as_i64().ok_or("no expiresAt")?;
    let now = chrono::Utc::now().timestamp_millis();
    let wait = u64::try_from(expires_at.saturating_sub(now)).unwrap_or(0) + 1_500;
    tokio::time::sleep(Duration::from_millis(wait)).await;
    assert!(verifier.verify(token).await.is_err());
    Ok(())
}

fn from_ip(mut request: Request<Body>, last_octet: u8) -> Request<Body> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, last_octet)), 40_000);
    request.extensions_mut().insert(ConnectInfo(address));
    request
}

fn forged_cookie() -> String {
    format!(
        "acme-session={}.{}",
        Uuid::new_v4().simple(),
        URL_SAFE_NO_PAD.encode([1_u8; 32])
    )
}

type Sent<'a> = Pin<Box<dyn Future<Output = Result<Response, TestError>> + Send + 'a>>;

async fn first_limited(
    attempts: usize,
    mut send_one: impl FnMut(usize) -> Sent<'static>,
) -> Result<Option<usize>, TestError> {
    for attempt in 1..=attempts {
        let response = send_one(attempt).await?;
        if response.status() == StatusCode::TOO_MANY_REQUESTS {
            assert!(response.headers().get(RETRY_AFTER).is_some());
            return Ok(Some(attempt));
        }
    }
    Ok(None)
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn redis_rate_limits_cap_exchange_per_ip_and_refresh_per_session() -> TestResult {
    let fixture = fixture(true).await?;
    let ip_capacity = 30 + 10;
    let session_capacity = 12 + 4;
    let bad_exchange = serde_json::json!({
        "code": "unused",
        "codeVerifier": "unused",
        "redirectUri": "https://evil.example/callback",
    });
    let app = fixture.app.clone();
    let limited = first_limited(ip_capacity + 1, |_| {
        let app = app.clone();
        let body = bad_exchange.clone();
        Box::pin(async move {
            send(
                &app,
                from_ip(session_request("exchange", None, Some(&body))?, 1),
            )
            .await
        })
    })
    .await?;
    assert_eq!(limited, Some(ip_capacity + 1));

    let cookie = forged_cookie();
    let limited = first_limited(session_capacity + 1, |attempt| {
        let app = app.clone();
        let cookie = cookie.clone();
        let octet = u8::try_from(attempt + 10).unwrap_or(u8::MAX);
        Box::pin(async move {
            send(
                &app,
                from_ip(session_request("refresh", Some(&cookie), None)?, octet),
            )
            .await
        })
    })
    .await?;
    assert_eq!(limited, Some(session_capacity + 1));

    let other = forged_cookie();
    let response = send(
        &app,
        from_ip(session_request("refresh", Some(&other), None)?, 11),
    )
    .await?;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    Ok(())
}
