//! Token-bucket request limiting and fixed-window amount budgets.
//!
//! [`RateLimitStore`] is the persistence port. [`RedisRateLimitStore`] provides
//! cross-instance atomic decisions, while [`InMemoryRateLimitStore`] is a
//! bounded adapter for tests and Redis-less local development. [`layers`] and
//! [`authenticated_route_group`] compose with [`baukit_http::layers`].
//! [`AmountBudget`] handles callers that consume a variable number of units.
//!
//! Authenticated limiting reads a verified [`baukit_auth::Principal`] from
//! request extensions. Place [`baukit_auth::establish_principal`] outside this
//! layer so the principal is available before the request is consumed.

#![deny(missing_docs)]

mod amount_budget;
mod axum_layer;
#[cfg(feature = "browser-session")]
mod browser_session;
pub mod kernels;
mod memory;
mod options;
#[cfg(test)]
mod redis_parity_tests;
mod redis_scripts;
mod redis_store;
mod store;

pub use amount_budget::{
    AmountBudget, AmountBudgetDecision, BudgetClock, BudgetWindow,
    FIXED_WINDOW_AMOUNT_BUDGET_DECISIONS_TOTAL, FixedWindow, FixedWindowAmountBudget,
    FixedWindowBudgetOptions, FixedWindowBudgetOptionsError, FixedWindowError, SystemBudgetClock,
};
pub use axum_layer::{
    HTTP_RATE_LIMIT_DECISIONS_TOTAL, RATE_LIMIT_LIMIT, RATE_LIMIT_REMAINING, RATE_LIMIT_RESET,
    authenticated_route_group, layers, resolve_client_ip,
};
pub use baukit_config::RateLimitFailMode;
pub use baukit_resilience::startup::StartupError;
#[cfg(feature = "browser-session")]
pub use browser_session::StoreBrowserSessionRateLimiter;
pub use kernels::client_ip::{ip_bucket, select_forwarded_ip};
pub use memory::{InMemoryRateLimitStore, InMemoryStoreError};
pub use options::{
    AuthenticatedRouteGroupOptions, AuthenticatedRouteGroupOptionsError, RateLimitOptions,
    RateLimitOptionsError, RateLimitScopeOptions,
};
pub use redis_store::RedisRateLimitStore;
pub use store::{
    AmountBudgetStore, AmountBudgetStoreDecision, Quota, QuotaError, RateLimitDecision,
    RateLimitStore, RateLimitStoreError, SharedRateLimitStore,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
