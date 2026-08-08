//! Shared integration fixtures and conformance assertions for Baukit services.
//!
//! The crate provides Docker-backed PostgreSQL fixtures, compact test tracing,
//! Prometheus contract checks, in-process or network operations-endpoint checks,
//! OpenAPI drift assertions, and small JWT fixtures.
//!
//! # Telemetry tests
//!
//! Full `baukit-telemetry` initialization is process-global and cannot be reset,
//! even after shutdown. Put all assertions requiring a real telemetry recorder,
//! subscriber, or exporter in one contract test for that test-binary process and
//! initialize telemetry exactly once there. [`init_test_tracing`] is only a
//! lightweight, exporter-free subscriber helper for tests that need log output;
//! it does not create an isolated telemetry runtime.
//!
//! ```no_run
//! use baukit_test::{JwtClaims, hs256_token, init_test_tracing};
//!
//! init_test_tracing();
//! let claims = JwtClaims::new()
//!     .subject("test-user")
//!     .issuer("test-suite")
//!     .audience("orders-api")
//!     .expires_at(4_102_444_800);
//! let token = hs256_token(b"test-only-secret", &claims)?;
//! assert_eq!(token.split('.').count(), 3);
//! # Ok::<(), jsonwebtoken::errors::Error>(())
//! ```

#![deny(missing_docs)]

mod jwt;
mod metrics;
mod ops;
mod postgres;
mod tracing;

pub use baukit_openapi::{
    SchemaError as OpenApiDriftError, assert_no_drift as assert_openapi_no_drift,
    check_no_drift as check_openapi_no_drift,
};
pub use jwt::{JwtClaims, authorization_header, hs256_token, rs256_token};
pub use metrics::{MetricsConformanceError, assert_metrics_conformance, check_metrics_conformance};
pub use ops::{
    OpsConformanceError, assert_ops_base_url_conformance, assert_ops_router_conformance,
    check_ops_base_url_conformance, check_ops_router_conformance,
};
#[cfg(feature = "sqlx-postgres")]
pub use postgres::start_postgres_with_migrations;
pub use postgres::{PostgresTestContainer, PostgresTestError, start_postgres};
pub use tracing::init_test_tracing;
