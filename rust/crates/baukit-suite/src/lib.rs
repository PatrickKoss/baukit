//! OAuth-style peer links and signed, transactional suite events.

pub mod adapters;
pub mod config;
pub mod domain;
pub mod ports;
mod secret_cipher;
pub mod services;

/// Copy this schema after the baukit-jobs migrations. Products add owner foreign keys.
/// The crate never runs migrations on startup.
pub const SUITE_MIGRATION_SQL: &str = include_str!("../migrations/001_suite.sql");
/// Copy after `SUITE_MIGRATION_SQL` to add replay, sealed exchange recovery and failure accounting.
pub const SUITE_RUNTIME_MIGRATION_SQL: &str = include_str!("../migrations/002_suite_runtime.sql");
