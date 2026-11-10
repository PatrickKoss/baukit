//! Browser session backend-for-frontend from ADR 0006.
//!
//! The browser never holds a refresh token. It still runs the authorization
//! code flow with PKCE against the IdP, then posts the code and verifier to
//! `POST /auth/session/exchange`. The backend redeems them, stores the refresh
//! and ID tokens encrypted in `baukit_auth_browser_sessions`, and sets an
//! `HttpOnly`, `SameSite=Strict` cookie holding `<session id>.<secret>`. The
//! browser keeps only the short-lived access token in memory and gets a new
//! one from `POST /auth/session/refresh`, which rotates the cookie. A rotated
//! cookie presented after a 10 second grace window revokes the whole family.
//! `POST /auth/session/logout` revokes the refresh token at the IdP and
//! returns the IdP's end-session URL when it has one.
//!
//! Every route requires `X-Baukit-Session: 1` and an `Origin` from the CORS
//! allowlist. Rate limiting goes through [`BrowserSessionRateLimiter`].
//!
//! ```no_run
//! use std::sync::Arc;
//!
//! use axum::Router;
//! use baukit_auth::browser_session::{
//!     self, BrowserSessionConfig, BrowserSessionRateLimiter, BrowserSessionState,
//! };
//! use baukit_credential_vault::CredentialCipher;
//! use baukit_http::HttpOptions;
//! use sqlx::PgPool;
//!
//! # async fn example(
//! #     pool: PgPool,
//! #     cipher: CredentialCipher,
//! #     limiter: Arc<dyn BrowserSessionRateLimiter>,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let mut http = HttpOptions::default().with_allowed_origins(["https://app.example.com"])?;
//! http.allow_credentials = true;
//! let config = BrowserSessionConfig::new(
//!     "https://identity.example.com/realms/products",
//!     "orders-web",
//!     "orders-session",
//!     "https://api.example.com",
//!     cipher,
//! )?
//! .with_redirect_uris(["https://app.example.com/auth/callback"]);
//! let state = BrowserSessionState::discover(config, &http, pool, limiter).await?;
//! let _app: Router = Router::new().merge(browser_session::router(state));
//! # Ok(())
//! # }
//! ```

mod config;
mod cookie;
mod crypto;
mod idp;
#[cfg(test)]
mod idp_tests;
mod rate_limit;
#[cfg(test)]
mod redirect_tests;
mod router;
mod service;
mod store;
#[cfg(test)]
mod tests;

pub use config::{BrowserSessionConfig, BrowserSessionConfigError};
pub use idp::IdpDiscoveryError;
pub use rate_limit::{
    AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BrowserSessionRateLimit,
    BrowserSessionRateLimiter, BrowserSessionRateLimits, RateLimitFuture, RateLimitOutcome,
    RateLimitSubject,
};
pub use router::{
    BROWSER_SESSION_HEADER, BROWSER_SESSION_PATH_PREFIX, BrowserSessionError, BrowserSessionState,
    router,
};
pub use store::{erase_subject_browser_sessions, purge_browser_sessions};

/// Reference PostgreSQL schema for `baukit_auth_browser_sessions`.
///
/// Copy this SQL into a product migration; baukit-auth does not migrate on startup.
pub const POSTGRES_BROWSER_SESSIONS_MIGRATION_SQL: &str =
    include_str!("../../migrations/0002_baukit_auth_browser_sessions.sql");
