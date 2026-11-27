use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State, connect_info::MockConnectInfo},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use baukit_http::ApiError;
use baukit_ratelimit::{
    Quota, QuotaError, RateLimitFailMode, RateLimitOptions, RateLimitStore, ip_bucket,
    resolve_client_ip,
};

use super::http::{SuiteHttpState, inbound_router, public_router, user_router};

const DEFAULT_REQUESTS_PER_MINUTE: u64 = 60;
const DEFAULT_BURST: u64 = 10;
const MINUTE: Duration = Duration::from_secs(60);
const DEFAULT_IPV6_PREFIX_LEN: u8 = 64;
const KEY_SCOPE: &str = "suite-public:ip:";

/// Per-client-IP bucket for the unauthenticated callback, exchange and revoke routes.
///
/// Signed inbound events keep their per-link and per-source buckets in the ingest service.
#[derive(Clone, Debug)]
pub struct SuitePublicRateLimit {
    pub quota: Quota,
    /// Reverse-proxy hops trusted in `X-Forwarded-For`, including the socket peer.
    pub trusted_proxy_hops: usize,
    /// IPv6 prefix length that shares one bucket.
    pub ipv6_prefix_len: u8,
    pub fail_mode: RateLimitFailMode,
    pub key_prefix: String,
}

impl SuitePublicRateLimit {
    /// 60 requests per minute and IP with a burst of 10, no trusted proxy hop, IPv6 per /64,
    /// fail closed.
    ///
    /// # Errors
    ///
    /// `QuotaError` when the default quota is rejected. The defaults are non-zero, so this does not
    /// happen in practice.
    pub fn new() -> Result<Self, QuotaError> {
        Ok(Self {
            quota: Quota::new(DEFAULT_REQUESTS_PER_MINUTE, MINUTE, DEFAULT_BURST)?,
            trusted_proxy_hops: 0,
            ipv6_prefix_len: DEFAULT_IPV6_PREFIX_LEN,
            fail_mode: RateLimitFailMode::Closed,
            key_prefix: "rl:".into(),
        })
    }

    /// Keeps the default quota and takes proxy trust, fail mode and key prefix from the product.
    ///
    /// # Errors
    ///
    /// `QuotaError` under the same condition as [`Self::new`].
    pub fn from_options(options: &RateLimitOptions) -> Result<Self, QuotaError> {
        Ok(Self {
            trusted_proxy_hops: options.trusted_proxy_hops,
            ipv6_prefix_len: options.ipv6_prefix_len,
            fail_mode: options.fail_mode,
            key_prefix: options.key_prefix().to_owned(),
            ..Self::new()?
        })
    }

    #[must_use]
    pub fn with_quota(self, quota: Quota) -> Self {
        Self { quota, ..self }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("request has no socket peer address for the suite public rate limit")]
struct MissingPeerAddress;

#[derive(Clone)]
struct LimitState {
    store: Arc<dyn RateLimitStore>,
    limit: SuitePublicRateLimit,
}

/// Suite router with the IP limit applied to the unauthenticated link routes.
pub fn limited_router(
    state: SuiteHttpState,
    store: Arc<dyn RateLimitStore>,
    limit: SuitePublicRateLimit,
) -> Router {
    public_router(state.clone())
        .layer(middleware::from_fn_with_state(
            LimitState { store, limit },
            limit_public_request,
        ))
        .merge(inbound_router(state.clone()))
        .merge(user_router(state))
}

/// The peer address the way axum's `ConnectInfo` extractor finds it, `MockConnectInfo` included.
fn peer_address(request: &Request) -> Option<SocketAddr> {
    let extensions = request.extensions();
    extensions
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ConnectInfo(peer)| *peer)
        .or_else(|| {
            extensions
                .get::<MockConnectInfo<SocketAddr>>()
                .map(|MockConnectInfo(peer)| *peer)
        })
}

async fn limit_public_request(
    State(state): State<LimitState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(peer) = peer_address(&request) else {
        tracing::error!(
            "suite public rate limit has no ConnectInfo<SocketAddr>; \
             serve the router with into_make_service_with_connect_info"
        );
        return ApiError::internal(MissingPeerAddress).into_response();
    };
    let Some(address) = resolve_client_ip(
        request.headers(),
        Some(peer),
        state.limit.trusted_proxy_hops,
    ) else {
        return ApiError::internal(MissingPeerAddress).into_response();
    };
    let bucket = match ip_bucket(address, state.limit.ipv6_prefix_len) {
        IpAddr::V4(address) => address.to_string(),
        IpAddr::V6(network) => format!("{network}/{}", state.limit.ipv6_prefix_len),
    };
    let key = format!("{}{KEY_SCOPE}{bucket}", state.limit.key_prefix);
    let retry_after = match state.store.check_and_consume(&key, state.limit.quota).await {
        Ok(decision) if decision.allowed => return next.run(request).await,
        Ok(decision) => decision.retry_after,
        Err(error) => {
            tracing::warn!(%error, "suite public rate-limit decision failed");
            if state.limit.fail_mode == RateLimitFailMode::Open {
                return next.run(request).await;
            }
            state.limit.quota.period()
        }
    };
    let seconds = retry_after
        .as_secs()
        .saturating_add(u64::from(retry_after.subsec_nanos() > 0));
    ApiError::rate_limited()
        .with_retry_after(seconds.max(1))
        .into_response()
}
