use std::{future::Future, net::SocketAddr, pin::Pin, time::Duration};

use axum::http::HeaderMap;

/// One token bucket from ADR 0006.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BrowserSessionRateLimit {
    /// Route group name, used in store keys and metrics.
    pub group: &'static str,
    /// Sustained requests per minute.
    pub requests_per_minute: u64,
    /// Extra requests allowed in a burst.
    pub burst: u64,
}

/// Per client IP bucket for exchange, refresh and logout.
pub const AUTH_SESSION_IP_LIMIT: BrowserSessionRateLimit = BrowserSessionRateLimit {
    group: "auth-session-ip",
    requests_per_minute: 30,
    burst: 10,
};

/// Per session id bucket for refresh.
pub const AUTH_SESSION_ID_LIMIT: BrowserSessionRateLimit = BrowserSessionRateLimit {
    group: "auth-session-id",
    requests_per_minute: 12,
    burst: 4,
};

/// What a bucket counts.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum RateLimitSubject<'a> {
    /// The client IP. The adapter resolves it from the headers and the socket
    /// peer with its trusted proxy depth.
    ClientIp {
        /// Request headers, for `X-Forwarded-For`.
        headers: &'a HeaderMap,
        /// The socket peer, when the server records `ConnectInfo`.
        peer: Option<SocketAddr>,
    },
    /// The session id part of the cookie. Never the secret.
    SessionId(&'a str),
}

/// Outcome of one bucket check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitOutcome {
    /// The request may proceed.
    Allowed,
    /// The request is refused; the response carries `Retry-After`.
    Limited {
        /// Time until the bucket admits another request.
        retry_after: Duration,
    },
}

/// Boxed future returned by [`BrowserSessionRateLimiter::check`].
pub type RateLimitFuture<'a> = Pin<Box<dyn Future<Output = RateLimitOutcome> + Send + 'a>>;

/// Rate-limit port for the browser session router.
///
/// `baukit-ratelimit` depends on `baukit-auth`, so the adapter over its shared
/// Redis store lives outside this crate. The adapter applies the store's fail
/// mode and returns [`RateLimitOutcome::Limited`] when it fails closed.
pub trait BrowserSessionRateLimiter: Send + Sync {
    /// Consumes one token from `limit`'s bucket for `subject`.
    fn check<'a>(
        &'a self,
        limit: BrowserSessionRateLimit,
        subject: RateLimitSubject<'a>,
    ) -> RateLimitFuture<'a>;
}
