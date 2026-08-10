//! Redis-backed identity and client-IP token-bucket limiting for Axum.
//!
//! [`RateLimitStore`] is the persistence port. [`RedisRateLimitStore`] provides
//! cross-instance atomic decisions, while [`InMemoryRateLimitStore`] is a
//! bounded adapter for tests and Redis-less local development. [`layers`]
//! composes with [`baukit_http::layers`].
//!
//! Authenticated limiting reads a verified [`baukit_auth::Principal`] from
//! request extensions. Place middleware that establishes the principal outside
//! this layer so it is available before the request is consumed.

#![deny(missing_docs)]

mod axum_layer;
mod memory;
mod options;
mod redis_store;
mod store;

pub use axum_layer::{
    HTTP_RATE_LIMIT_DECISIONS_TOTAL, RATE_LIMIT_LIMIT, RATE_LIMIT_REMAINING, RATE_LIMIT_RESET,
    layers, resolve_client_ip,
};
pub use baukit_config::RateLimitFailMode;
pub use memory::{InMemoryRateLimitStore, InMemoryStoreError};
pub use options::{RateLimitOptions, RateLimitOptionsError, RateLimitScopeOptions};
pub use redis_store::RedisRateLimitStore;
pub use store::{Quota, QuotaError, RateLimitDecision, RateLimitStore, RateLimitStoreError};
