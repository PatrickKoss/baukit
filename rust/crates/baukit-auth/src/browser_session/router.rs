use std::{net::SocketAddr, sync::Arc};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{ConnectInfo, FromRequestParts, State},
    http::{
        HeaderMap, HeaderName, HeaderValue, StatusCode,
        header::{CACHE_CONTROL, ORIGIN, RETRY_AFTER, SET_COOKIE},
        request::Parts,
    },
    middleware,
    response::{IntoResponse, Response},
    routing::post,
};
use baukit_http::ApiError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use super::{
    config::{BrowserSessionConfig, BrowserSessionConfigError},
    cookie::CookieSettings,
    idp::{IdentityProvider, IdpDiscoveryError, OidcTokenClient},
    rate_limit::{
        AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BrowserSessionRateLimit,
        BrowserSessionRateLimiter, RateLimitOutcome, RateLimitSubject,
    },
    service::{Clock, Issued, SessionError, SessionService, SessionSettings, SystemClock},
    store::{PostgresSessionStore, SessionStore},
};

/// Reserved path prefix of the browser session routes.
pub const BROWSER_SESSION_PATH_PREFIX: &str = "/auth/session";

/// Header every browser session request must carry with the value `1`.
pub const BROWSER_SESSION_HEADER: HeaderName = HeaderName::from_static("x-baukit-session");

/// Shared state of the browser session router.
#[derive(Clone)]
pub struct BrowserSessionState {
    inner: Arc<StateInner>,
}

struct StateInner {
    service: SessionService,
    cookie: CookieSettings,
    allowed_origins: Vec<String>,
    rate_limiter: Arc<dyn BrowserSessionRateLimiter>,
}

impl std::fmt::Debug for BrowserSessionState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrowserSessionState")
            .field("cookie", &self.inner.cookie.name())
            .field("allowed_origins", &self.inner.allowed_origins)
            .finish_non_exhaustive()
    }
}

/// Failure to build the browser session router. Startup must fail.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BrowserSessionError {
    /// The configuration is invalid for the HTTP options.
    #[error(transparent)]
    Config(#[from] BrowserSessionConfigError),
    /// IdP discovery failed.
    #[error(transparent)]
    Discovery(#[from] IdpDiscoveryError),
}

impl BrowserSessionState {
    /// Validates `config` against `http`, discovers the IdP and builds the state.
    ///
    /// `http` must be the options the product passes to `baukit_http::layers`,
    /// so the CSRF origin check uses the CORS allowlist. Startup fails when that
    /// allowlist is empty, `allow_credentials` is off, or any check in
    /// [`BrowserSessionConfig::validate`] fails.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionError::Config`] when [`BrowserSessionConfig::validate`]
    /// fails, and [`BrowserSessionError::Discovery`] when the IdP egress client
    /// cannot be built or the issuer's discovery fails.
    pub async fn discover(
        config: BrowserSessionConfig,
        http: &baukit_http::HttpOptions,
        pool: PgPool,
        rate_limiter: Arc<dyn BrowserSessionRateLimiter>,
    ) -> Result<Self, BrowserSessionError> {
        config.validate(http)?;
        let idp = OidcTokenClient::discover(&config).await?;
        Self::from_parts(
            config,
            http,
            Arc::new(PostgresSessionStore::new(pool)),
            idp,
            Arc::new(SystemClock),
            rate_limiter,
        )
    }

    pub(crate) fn from_parts(
        config: BrowserSessionConfig,
        http: &baukit_http::HttpOptions,
        store: Arc<dyn SessionStore>,
        idp: Arc<dyn IdentityProvider>,
        clock: Arc<dyn Clock>,
        rate_limiter: Arc<dyn BrowserSessionRateLimiter>,
    ) -> Result<Self, BrowserSessionError> {
        let allowed_origins = config
            .validate(http)?
            .iter()
            .map(|origin| origin.origin().ascii_serialization())
            .collect();
        let cookie = CookieSettings::new(config.cookie_name(), !config.insecure_local_cookie);
        let settings = SessionSettings {
            cipher: config.cipher,
            redirect_uris: config.redirect_uris,
            idle_timeout: config.idle_timeout,
            absolute_timeout: config.absolute_timeout,
        };
        Ok(Self {
            inner: Arc::new(StateInner {
                service: SessionService::new(settings, store, idp, clock),
                cookie,
                allowed_origins,
                rate_limiter,
            }),
        })
    }
}

/// Builds `POST /auth/session/{exchange,refresh,logout}`.
///
/// Merge the result into the product router. Every response carries
/// `Cache-Control: no-store`.
pub fn router<S>(state: BrowserSessionState) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route(
            &format!("{BROWSER_SESSION_PATH_PREFIX}/exchange"),
            post(exchange),
        )
        .route(
            &format!("{BROWSER_SESSION_PATH_PREFIX}/refresh"),
            post(refresh),
        )
        .route(
            &format!("{BROWSER_SESSION_PATH_PREFIX}/logout"),
            post(logout),
        )
        .layer(middleware::map_response(no_store))
        .with_state(state)
}

async fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// The socket peer, when the server records `ConnectInfo`.
pub(crate) struct Peer(Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for Peer {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            parts
                .extensions
                .get::<ConnectInfo<SocketAddr>>()
                .map(|info| info.0),
        ))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExchangeRequest {
    code: String,
    code_verifier: String,
    redirect_uri: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    access_token: String,
    expires_at: i64,
    subject: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LogoutResponse {
    end_session_url: String,
}

async fn exchange(
    State(state): State<BrowserSessionState>,
    Peer(peer): Peer,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Some(response) = state.rejection(&headers, peer, None).await {
        return response;
    }
    let Ok(request) = serde_json::from_slice::<ExchangeRequest>(&body) else {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid_grant",
            "The request body is invalid.",
        );
    };
    let issued = state
        .inner
        .service
        .exchange(&request.code, &request.code_verifier, &request.redirect_uri)
        .await;
    state.issued_response(issued)
}

async fn refresh(
    State(state): State<BrowserSessionState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Response {
    let credential = state.inner.cookie.read(&headers);
    let session_id = credential
        .as_ref()
        .map(|credential| credential.id.simple().to_string());
    if let Some(response) = state.rejection(&headers, peer, session_id.as_deref()).await {
        return response;
    }
    let Some(credential) = credential else {
        return state.expired();
    };
    let issued = state.inner.service.refresh(&credential).await;
    state.issued_response(issued)
}

async fn logout(
    State(state): State<BrowserSessionState>,
    Peer(peer): Peer,
    headers: HeaderMap,
) -> Response {
    if let Some(response) = state.rejection(&headers, peer, None).await {
        return response;
    }
    let credential = state.inner.cookie.read(&headers);
    let outcome = state.inner.service.logout(credential.as_ref()).await;
    let response = match outcome {
        Ok(Some(end_session_url)) => Json(LogoutResponse { end_session_url }).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => state.error_response(error),
    };
    state.with_cleared_cookie(response)
}

impl BrowserSessionState {
    /// Returns the response refusing the request, or `None` when it may proceed.
    async fn rejection(
        &self,
        headers: &HeaderMap,
        peer: Option<SocketAddr>,
        session_id: Option<&str>,
    ) -> Option<Response> {
        if !self.csrf_allows(headers) {
            return Some(problem(
                StatusCode::FORBIDDEN,
                "csrf_rejected",
                "The request did not come from an allowed origin.",
            ));
        }
        let ip = RateLimitSubject::ClientIp { headers, peer };
        if let Some(limited) = self.limited(AUTH_SESSION_IP_LIMIT, ip).await {
            return Some(limited);
        }
        let session_id = session_id?;
        self.limited(
            AUTH_SESSION_ID_LIMIT,
            RateLimitSubject::SessionId(session_id),
        )
        .await
    }

    fn csrf_allows(&self, headers: &HeaderMap) -> bool {
        let marked = headers
            .get(BROWSER_SESSION_HEADER)
            .is_some_and(|value| value.as_bytes() == b"1");
        let allowed_origin = headers
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|origin| {
                self.inner
                    .allowed_origins
                    .iter()
                    .any(|allowed| allowed == origin)
            });
        marked && allowed_origin
    }

    async fn limited(
        &self,
        limit: BrowserSessionRateLimit,
        subject: RateLimitSubject<'_>,
    ) -> Option<Response> {
        match self.inner.rate_limiter.check(limit, subject).await {
            RateLimitOutcome::Allowed => None,
            RateLimitOutcome::Limited { retry_after } => {
                let seconds = retry_after.as_secs().max(1);
                let mut response = problem(
                    StatusCode::TOO_MANY_REQUESTS,
                    "rate_limited",
                    "Too many session requests.",
                );
                response
                    .headers_mut()
                    .insert(RETRY_AFTER, HeaderValue::from(seconds));
                Some(response)
            }
        }
    }

    fn issued_response(&self, issued: Result<Issued, SessionError>) -> Response {
        let issued = match issued {
            Ok(issued) => issued,
            Err(SessionError::SessionExpired) => return self.expired(),
            Err(error) => return self.error_response(error),
        };
        let body = SessionResponse {
            access_token: issued.access_token.to_string(),
            expires_at: issued.expires_at.timestamp_millis(),
            subject: issued.subject,
        };
        let mut response = Json(body).into_response();
        if let Some((credential, max_age)) = &issued.cookie {
            let Some(cookie) = self.inner.cookie.set(credential, *max_age) else {
                return problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal_error",
                    "The session cookie could not be built.",
                );
            };
            response.headers_mut().insert(SET_COOKIE, cookie);
        }
        response
    }

    fn expired(&self) -> Response {
        self.with_cleared_cookie(problem(
            StatusCode::UNAUTHORIZED,
            "session_expired",
            "The browser session has expired.",
        ))
    }

    fn with_cleared_cookie(&self, mut response: Response) -> Response {
        if let Some(cookie) = self.inner.cookie.clear() {
            response.headers_mut().append(SET_COOKIE, cookie);
        }
        response
    }

    fn error_response(&self, error: SessionError) -> Response {
        match error {
            SessionError::SessionExpired => self.expired(),
            SessionError::InvalidGrant => problem(
                StatusCode::BAD_REQUEST,
                "invalid_grant",
                "The authorization grant was rejected.",
            ),
            SessionError::IdpUnavailable(_) => {
                tracing::warn!(error = %error, "browser session IdP call failed");
                problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "idp_unavailable",
                    "The identity provider is unavailable. Retry later.",
                )
            }
            SessionError::InvalidIdpResponse(_) => {
                tracing::warn!(error = %error, "browser session IdP response failed verification");
                problem(
                    StatusCode::BAD_GATEWAY,
                    "idp_invalid_response",
                    "The identity provider returned an invalid response.",
                )
            }
            SessionError::Store(_) | SessionError::Crypto(_) => {
                ApiError::internal(error).into_response()
            }
        }
    }
}

fn problem(status: StatusCode, code: &'static str, message: &'static str) -> Response {
    ApiError::new(status, code, message).into_response()
}
