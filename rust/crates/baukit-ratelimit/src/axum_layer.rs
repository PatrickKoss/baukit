use std::{collections::BTreeMap, net::IpAddr, net::SocketAddr, sync::Arc, time::Duration};

use axum::{
    Router,
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware::{self, Next},
    response::{IntoResponse as _, Response},
};
use baukit_auth::Principal;

use crate::{
    AuthenticatedRouteGroupOptions, Quota, RateLimitDecision, RateLimitFailMode, RateLimitOptions,
    RateLimitScopeOptions, RateLimitStore, kernels::client_ip::select_forwarded_ip,
};

/// Counter for rate-limit decisions, labeled exactly `scope` and `outcome`.
pub const HTTP_RATE_LIMIT_DECISIONS_TOTAL: &str = "http_rate_limit_decisions_total";
/// IETF rate-limit policy limit header.
pub const RATE_LIMIT_LIMIT: HeaderName = HeaderName::from_static("ratelimit-limit");
/// IETF rate-limit remaining-quota header.
pub const RATE_LIMIT_REMAINING: HeaderName = HeaderName::from_static("ratelimit-remaining");
/// IETF rate-limit reset-delay header.
pub const RATE_LIMIT_RESET: HeaderName = HeaderName::from_static("ratelimit-reset");

const X_FORWARDED_FOR: HeaderName = HeaderName::from_static("x-forwarded-for");

#[derive(Debug, thiserror::Error)]
#[error("request has no socket peer address for IP rate limiting")]
struct MissingPeerAddress;

#[derive(Clone)]
struct LayerState {
    store: Arc<dyn RateLimitStore>,
    options: RateLimitOptions,
}

type SubjectKeyExtractor = dyn Fn(&Principal) -> String + Send + Sync;
type RequestPredicate = dyn Fn(&Request) -> bool + Send + Sync;

#[derive(Clone)]
struct AuthenticatedRouteGroupState {
    store: Arc<dyn RateLimitStore>,
    options: AuthenticatedRouteGroupOptions,
    subject_key: Arc<SubjectKeyExtractor>,
    request_predicate: Arc<RequestPredicate>,
}

/// Applies identity-first and client-IP rate limiting to an Axum router.
///
/// Compose this function independently with [`baukit_http::layers`]. A verified
/// [`Principal`] already present in request extensions activates the identity
/// scope; the IP scope is evaluated for every request when enabled.
pub fn layers<S, Store>(router: Router<S>, store: Store, options: RateLimitOptions) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Store: RateLimitStore + 'static,
{
    router.layer(middleware::from_fn_with_state(
        LayerState {
            store: Arc::new(store),
            options,
        },
        rate_limit_request,
    ))
}

/// Applies one named token bucket to selected requests with a verified principal.
///
/// `subject_key` derives an opaque counter key from the principal. The key is
/// used only in the store and never as a metric label. `request_predicate`
/// selects which requests consume the group bucket. Requests without a cached
/// principal pass through so the route's authentication policy can reject them.
pub fn authenticated_route_group<S, Store, SubjectKey, Predicate>(
    router: Router<S>,
    store: Store,
    options: AuthenticatedRouteGroupOptions,
    subject_key: SubjectKey,
    request_predicate: Predicate,
) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
    Store: RateLimitStore + 'static,
    SubjectKey: Fn(&Principal) -> String + Send + Sync + 'static,
    Predicate: Fn(&Request) -> bool + Send + Sync + 'static,
{
    router.layer(middleware::from_fn_with_state(
        AuthenticatedRouteGroupState {
            store: Arc::new(store),
            options,
            subject_key: Arc::new(subject_key),
            request_predicate: Arc::new(request_predicate),
        },
        rate_limit_authenticated_route_group,
    ))
}

async fn rate_limit_authenticated_route_group(
    State(state): State<AuthenticatedRouteGroupState>,
    request: Request,
    next: Next,
) -> Response {
    if !(state.request_predicate)(&request) {
        return next.run(request).await;
    }
    let Some(principal) = request.extensions().get::<Principal>() else {
        return next.run(request).await;
    };
    let key = state.options.key(&(state.subject_key)(principal));
    match state
        .store
        .check_and_consume(&key, state.options.quota)
        .await
    {
        Ok(decision) if decision.allowed => {
            record(state.options.group(), "allowed");
            next.run(request).await
        }
        Ok(decision) => {
            record(state.options.group(), "limited");
            limited_response(state.options.quota, decision)
        }
        Err(error) => {
            record(state.options.group(), "error");
            tracing::warn!(
                group = state.options.group(),
                error = %error,
                "authenticated route-group rate-limit store decision failed"
            );
            match state.options.fail_mode {
                RateLimitFailMode::Open => next.run(request).await,
                RateLimitFailMode::Closed => limited_response(
                    state.options.quota,
                    RateLimitDecision {
                        allowed: false,
                        remaining: 0,
                        retry_after: state.options.quota.period(),
                    },
                ),
            }
        }
    }
}

async fn rate_limit_request(
    State(state): State<LayerState>,
    request: Request,
    next: Next,
) -> Response {
    let mut response_policy = None;

    if state.options.identity.enabled
        && let Some(principal) = request.extensions().get::<Principal>()
    {
        let key = state.options.identity_key(principal.subject());
        match evaluate(&state, "identity", &key, state.options.identity).await {
            Evaluation::Allowed(decision) => {
                response_policy = Some((state.options.identity, decision))
            }
            Evaluation::PassAfterError => {}
            Evaluation::Rejected(response) => return response,
        }
    }

    if state.options.ip.enabled {
        let Some(ConnectInfo(peer)) = request
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .copied()
        else {
            record("ip", "error");
            tracing::error!(
                "IP rate limit is enabled but the request has no ConnectInfo<SocketAddr>; \
                 serve the router with into_make_service_with_connect_info"
            );
            return baukit_http::ApiError::internal(MissingPeerAddress).into_response();
        };
        let address = resolve_client_ip(
            request.headers(),
            Some(peer),
            state.options.trusted_proxy_hops,
        );
        let key = state.options.ip_key(address);
        match evaluate(&state, "ip", &key, state.options.ip).await {
            Evaluation::Allowed(decision) => {
                if response_policy.is_none() {
                    response_policy = Some((state.options.ip, decision));
                }
            }
            Evaluation::PassAfterError => {}
            Evaluation::Rejected(response) => return response,
        }
    }

    let mut response = next.run(request).await;
    if let Some((scope, decision)) = response_policy
        && !response.headers().contains_key(RATE_LIMIT_LIMIT)
    {
        insert_rate_limit_headers(
            response.headers_mut(),
            scope.quota,
            decision,
            scope.quota.period(),
        );
    }
    response
}

enum Evaluation {
    Allowed(RateLimitDecision),
    PassAfterError,
    Rejected(Response),
}

async fn evaluate(
    state: &LayerState,
    scope: &'static str,
    key: &str,
    options: RateLimitScopeOptions,
) -> Evaluation {
    match state.store.check_and_consume(key, options.quota).await {
        Ok(decision) if decision.allowed => {
            record(scope, "allowed");
            Evaluation::Allowed(decision)
        }
        Ok(decision) => {
            record(scope, "limited");
            Evaluation::Rejected(limited_response(options.quota, decision))
        }
        Err(error) => {
            record(scope, "error");
            tracing::warn!(scope, error = %error, "rate-limit store decision failed");
            match state.options.fail_mode {
                RateLimitFailMode::Open => Evaluation::PassAfterError,
                RateLimitFailMode::Closed => Evaluation::Rejected(limited_response(
                    options.quota,
                    RateLimitDecision {
                        allowed: false,
                        remaining: 0,
                        retry_after: options.quota.period(),
                    },
                )),
            }
        }
    }
}

fn limited_response(quota: Quota, decision: RateLimitDecision) -> Response {
    let retry_after = duration_seconds_ceil(decision.retry_after);
    let mut response = baukit_http::ApiError::rate_limited()
        .with_details(BTreeMap::from([(
            "retryAfter".to_owned(),
            retry_after.into(),
        )]))
        .with_retry_after(retry_after)
        .into_response();
    insert_rate_limit_headers(
        response.headers_mut(),
        quota,
        decision,
        decision.retry_after,
    );
    response
}

fn insert_rate_limit_headers(
    headers: &mut HeaderMap,
    quota: Quota,
    decision: RateLimitDecision,
    reset: Duration,
) {
    headers.insert(RATE_LIMIT_LIMIT, number_header(quota.capacity()));
    headers.insert(RATE_LIMIT_REMAINING, number_header(decision.remaining));
    headers.insert(
        RATE_LIMIT_RESET,
        number_header(duration_seconds_ceil(reset)),
    );
}

fn number_header(value: u64) -> HeaderValue {
    HeaderValue::from(value)
}

fn duration_seconds_ceil(duration: Duration) -> u64 {
    duration
        .as_secs()
        .saturating_add(u64::from(duration.subsec_nanos() != 0))
}

fn record(scope: &str, outcome: &'static str) {
    metrics::counter!(
        HTTP_RATE_LIMIT_DECISIONS_TOTAL,
        "scope" => scope.to_owned(),
        "outcome" => outcome
    )
    .increment(1);
}

/// Resolves a client IP without trusting more of `X-Forwarded-For` than configured.
///
/// `trusted_proxy_hops` counts the socket peer. One selects the rightmost XFF entry
/// and zero ignores XFF. Only the trusted entry is parsed (see
/// [`select_forwarded_ip`](crate::select_forwarded_ip)); a missing peer, a chain
/// shorter than the trust depth, or an unparseable trusted entry yields the peer.
#[must_use]
pub fn resolve_client_ip(
    headers: &HeaderMap,
    peer: Option<SocketAddr>,
    trusted_proxy_hops: usize,
) -> Option<IpAddr> {
    let entries: Vec<&str> = headers
        .get_all(&X_FORWARDED_FOR)
        .iter()
        .flat_map(|value| value.as_bytes().split(|byte| *byte == b','))
        .map(|entry| std::str::from_utf8(entry).unwrap_or_default())
        .collect();
    select_forwarded_ip(
        &entries,
        peer.map(|address| address.ip()),
        trusted_proxy_hops,
    )
}

#[cfg(test)]
mod tests;
