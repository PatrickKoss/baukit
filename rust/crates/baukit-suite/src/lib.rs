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
//! | `platform` | Hub receiver, tickets and entitlement caches | None |
//! | `jobs` | Job handler and hourly cleanup | `runtime` |
//!
//! Enable `http`, `jobs` and `delivery` for all supplied production adapters.
//! Ports that accept SQLx connections require `postgres`.
//! `SuiteConfig::validate_for` requires `runtime`; `SuiteConfig::registry` does not.

//!
//! # PostgreSQL lock order
//!
//! Acquire suite owner advisory locks first, in ascending owner UUID order when
//! a transaction touches several owners. Lock the product owner row through
//! `SuiteEventApplier::lock_owner` or the erasure owner hook next. Lock existing
//! suite links in ascending `id` order, then lock jobs. Requests and codes also
//! require the owner advisory lock before their row locks. Product writers call
//! `SuiteEventOutbox::lock_owner_in_transaction` before locking product rows.
//!
//! Cleanup takes a transaction-scoped `pg_try_advisory_xact_lock` for the whole
//! run before any owner locks. Another replica skips that run. Cleanup locks owners
//! only for rows eligible for removal.
//!
//! Each finished delivery attempt updates its link under the owner/link order.
//! A failed attempt adds one to `consecutive_failures`; at
//! [`domain::SUITE_CIRCUIT_FAILURES`] the link is disabled and probed after one
//! hour, then after a delay that doubles up to a day. A delivered probe makes the
//! link healthy again. Attempts the egress breaker refuses are deferred and not
//! counted. Apply `SUITE_DELIVERY_PROBES_MIGRATION_SQL` and then
//! `SUITE_VALIDATE_LINK_CHECKS_MIGRATION_SQL` after the record ids migration to
//! install this behavior.

pub mod adapters;
#[cfg(feature = "delivery")]
pub mod compose;
pub mod config;
pub mod domain;
mod error_codes;
#[cfg(feature = "platform")]
pub mod platform;
pub mod ports;
#[cfg(all(test, feature = "runtime"))]
mod record_tests;
#[cfg(feature = "runtime")]
mod secret_cipher;
pub mod services;

pub use domain::SUITE_JOB_TYPES;
pub use error_codes::SUITE_ERROR_CODES;

/// Copy this schema after the baukit-jobs migrations. Products add owner foreign keys.
/// The crate never runs migrations on startup.
#[cfg(feature = "postgres")]
pub const SUITE_MIGRATION_SQL: &str = include_str!("../migrations/001_suite.sql");
/// Copy after `SUITE_MIGRATION_SQL` to add replay, sealed exchange recovery and failure accounting.
#[cfg(feature = "postgres")]
pub const SUITE_RUNTIME_MIGRATION_SQL: &str = include_str!("../migrations/002_suite_runtime.sql");
/// Copy after `SUITE_RUNTIME_MIGRATION_SQL` to remove job/link lock inversion.
#[cfg(feature = "postgres")]
pub const SUITE_LOCK_ORDER_MIGRATION_SQL: &str =
    include_str!("../migrations/003_suite_lock_order.sql");
/// Copy after `004_platform.sql` to store vault v2 record ids next to sealed secrets.
#[cfg(feature = "postgres")]
pub const SUITE_RECORD_IDS_MIGRATION_SQL: &str =
    include_str!("../migrations/005_suite_record_ids.sql");
/// Copy after `SUITE_RECORD_IDS_MIGRATION_SQL`. It drops the SQL failure accounting
/// and adds the probe schedule for disabled links.
#[cfg(feature = "postgres")]
pub const SUITE_DELIVERY_PROBES_MIGRATION_SQL: &str =
    include_str!("../migrations/006_suite_delivery_probes.sql");
/// Copy after `SUITE_DELIVERY_PROBES_MIGRATION_SQL`. It validates the link checks that
/// migration adds `NOT VALID`.
#[cfg(feature = "postgres")]
pub const SUITE_VALIDATE_LINK_CHECKS_MIGRATION_SQL: &str =
    include_str!("../migrations/007_suite_validate_link_checks.sql");

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
