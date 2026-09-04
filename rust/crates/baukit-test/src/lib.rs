//! Shared integration fixtures and conformance assertions for Baukit services.
//!
//! The crate provides Docker-backed PostgreSQL, direct Redis, and Redis Sentinel
//! fixtures, compact test tracing, Prometheus contract checks, in-process or network
//! operations-endpoint checks, OpenAPI drift assertions, resource-limit conformance checks,
//! and a mock OIDC/JWKS issuer with JWT fixtures.
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
//! # Ok::<(), baukit_test::JwtFixtureError>(())
//! ```

#![deny(missing_docs)]

mod api_token;
mod auth;
mod connector;
mod credential_probe;
mod erasure;
#[cfg(test)]
mod fixture_tests;
mod inbox;
mod jwt;
mod limits;
mod live_row_cap;
mod metrics;
mod ops;
mod postgres;
mod redis;
mod tracing;
mod webhook;

pub use api_token::InMemoryApiTokenStore;
pub use auth::{
    AuthConformanceError, assert_auth_router_conformance, check_auth_router_conformance,
};
pub use baukit_openapi::{
    SchemaError as OpenApiDriftError, assert_no_drift as assert_openapi_no_drift,
    check_no_drift as check_openapi_no_drift,
};
pub use connector::{
    FAKE_SIGNATURE_HEADER, FakeConnector, FakeConnectorResponse, FakeConnectorScenario,
    UnknownScenario,
};
pub use credential_probe::{
    CredentialProbeConformanceCases, CredentialProbeConformanceError, ScriptedCredentialProbeHttp,
    ScriptedCredentialProbeResponse, assert_credential_probe_conformance,
    check_credential_probe_conformance,
};
pub use erasure::{
    CleanupKind, ErasureConformanceError, OwnedResourceCheck, ProductProfileErasureAdapter,
    check_product_profile_erasure_conformance,
};
pub use inbox::{
    InboxConformanceCases, InboxConformanceError, InboxDelivery, InboxDisposition, InboxFault,
    InboxFuture, InboxReceipt, InboxScope, InboxState, PostgresInboxPort,
    assert_postgres_inbox_conformance, check_postgres_inbox_conformance,
};
pub use jwt::{
    JwtClaims, JwtFixtureError, MockOidcServer, MockOidcSession, authorization_header, hs256_token,
    rs256_token, rs256_token_with_key_id, unsigned_token,
};
pub use limits::{
    LimitsConformanceError, LiveRowLimitAdapter, NamedIngress, NamedOutput,
    assert_ingress_reason_code_parity, assert_limit_boundaries, assert_reason_code_conformance,
    assert_soft_delete_capacity_reuse, assert_update_at_capacity, check_ingress_reason_code_parity,
    check_limit_boundaries, check_reason_code_conformance, check_soft_delete_capacity_reuse,
    check_update_at_capacity, compact_document_bytes, trimmed_text_length,
};
pub use live_row_cap::{
    LiveRowCapConformanceCases, LiveRowCapConformanceError, PostgresLiveRowCapAdapter,
    assert_postgres_live_row_cap_conformance, check_postgres_live_row_cap_conformance,
};
pub use metrics::{
    MetricsConformanceError, MetricsConformanceOptions, assert_metrics_conformance,
    assert_metrics_conformance_with_options, check_metrics_conformance,
    check_metrics_conformance_with_options,
};
pub use ops::{
    OpsConformanceError, assert_ops_base_url_conformance, assert_ops_router_conformance,
    check_ops_base_url_conformance, check_ops_router_conformance,
};
#[cfg(feature = "sqlx-postgres")]
pub use postgres::{
    ForeignKeyDeleteMismatch, audit_user_root_foreign_keys, start_postgres_with_migrations,
};
pub use postgres::{PostgresTestContainer, PostgresTestError, start_postgres};
pub use redis::{
    RedisSentinelTestContainer, RedisTestContainer, RedisTestError, start_redis,
    start_redis_sentinel,
};
pub use tracing::init_test_tracing;
pub use webhook::{
    MAX_SCRIPTED_WEBHOOK_BODY_BYTES, ReceivedWebhookRequest, ScriptedWebhookReceiver,
    ScriptedWebhookResponse, sign_webhook_hmac_sha256, verify_webhook_hmac_sha256,
    webhook_signing_input,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
