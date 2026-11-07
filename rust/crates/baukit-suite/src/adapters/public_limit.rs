use std::{net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use baukit_http::ApiError;
use baukit_ratelimit::{
    Quota, QuotaError, RateLimitFailMode, RateLimitOptions, RateLimitStore, resolve_client_ip,
};

use super::http::{SuiteHttpState, inbound_router, public_router, user_router};

const DEFAULT_REQUESTS_PER_MINUTE: u64 = 60;
const DEFAULT_BURST: u64 = 10;
const MINUTE: Duration = Duration::from_secs(60);
const KEY_SCOPE: &str = "suite-public:ip:";

/// Per-client-IP bucket for the unauthenticated callback, exchange and revoke routes.
///
/// Signed inbound events keep their per-link and per-source buckets in the ingest service.
#[derive(Clone, Debug)]
pub struct SuitePublicRateLimit {
    pub quota: Quota,
    /// Reverse-proxy hops trusted in `X-Forwarded-For`, including the socket peer.
    pub trusted_proxy_hops: usize,
    pub fail_mode: RateLimitFailMode,
    pub key_prefix: String,
}

impl SuitePublicRateLimit {
    /// 60 requests per minute and IP with a burst of 10, one trusted hop, fail closed.
    ///
    /// # Errors
    ///
    /// `QuotaError` when the default quota is rejected. The defaults are non-zero, so this does not
    /// happen in practice.
    pub fn new() -> Result<Self, QuotaError> {
        Ok(Self {
            quota: Quota::new(DEFAULT_REQUESTS_PER_MINUTE, MINUTE, DEFAULT_BURST)?,
            trusted_proxy_hops: 1,
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

async fn limit_public_request(
    State(state): State<LimitState>,
    request: Request,
    next: Next,
) -> Response {
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|info| info.0);
    let address = resolve_client_ip(request.headers(), peer, state.limit.trusted_proxy_hops)
        .map_or_else(|| "unknown".to_owned(), |address| address.to_string());
    let key = format!("{}{KEY_SCOPE}{address}", state.limit.key_prefix);
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
