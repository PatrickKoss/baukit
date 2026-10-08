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
//! only for rows eligible for removal or failure accounting. Failed-job triggers
//! only record job ids; they never lock links. The store accounts those records
//! under the owner/link/job order before reading health, changing preferences,
//! resetting a circuit or enqueueing work. Apply `SUITE_LOCK_ORDER_MIGRATION_SQL`
//! after the runtime migration to install this behavior.

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
/// Copy after `SUITE_RUNTIME_MIGRATION_SQL` to remove job/link lock inversion.
#[cfg(feature = "postgres")]
pub const SUITE_LOCK_ORDER_MIGRATION_SQL: &str =
    include_str!("../migrations/003_suite_lock_order.sql");
