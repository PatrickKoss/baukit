//! OAuth-style peer links and signed, transactional suite events.
//!
//! The default build includes peer registries, payload validation, link protocol,
//! serde contracts and configuration without database, Tokio or HTTP dependencies.
//!
//! | Feature | Adds | Enables |
//! | --- | --- | --- |
//! | `postgres` | SQLx ports, stores, migrations and erasure adapter. Stores use the jobs outbox. | None |
//! | `runtime` | Link, ingest, delivery and erasure services, credential validation and rate limits | `postgres` |
//! | `delivery` | Guarded `ReqwestSuitePeerClient` | `runtime` |
//! | `http` | Axum router and OpenAPI definitions | `runtime` |
//! | `jobs` | Job handler and hourly cleanup | `runtime` |
//!
//! Enable `http`, `jobs` and `delivery` for all supplied production adapters.
//! Ports that accept SQLx connections require `postgres`.
//! `SuiteConfig::validate_for` requires `runtime`; `SuiteConfig::registry` does not.

pub mod adapters;
pub mod config;
pub mod domain;
pub mod ports;
#[cfg(feature = "runtime")]
mod secret_cipher;
pub mod services;

/// Copy this schema after the baukit-jobs migrations. Products add owner foreign keys.
/// The crate never runs migrations on startup.
#[cfg(feature = "postgres")]
pub const SUITE_MIGRATION_SQL: &str = include_str!("../migrations/001_suite.sql");
/// Copy after `SUITE_MIGRATION_SQL` to add replay, sealed exchange recovery and failure accounting.
#[cfg(feature = "postgres")]
pub const SUITE_RUNTIME_MIGRATION_SQL: &str = include_str!("../migrations/002_suite_runtime.sql");
