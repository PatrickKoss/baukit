//! Revision allocation, tombstone purge horizons, and hybrid logical clocks
//! for incremental sync.
//!
//! A syncable row carries a `revision` drawn from a counter that is private to
//! its owner. A client pulls by asking for everything above the revision it
//! last saw, so the counter must be monotonic per owner and must move in the
//! same transaction as the row write it stamps. [`next_revision`] does exactly
//! that. The [`purge`] module deletes old tombstones and records the per-owner
//! purge horizon, and [`horizon`] holds the pull-cursor rule that turns a
//! cursor below the horizon into a full-resync signal. The [`hlc`] module
//! supplies a cross-runtime logical clock for ordering writes when physical
//! clocks stall or move backward.
//!
//! # Features
//!
//! Without features the crate contains only [`hlc`], [`horizon`], and the
//! reference SQL constants, and depends on no database driver. The
//! `sqlx-postgres` feature adds the PostgreSQL revision allocator and the
//! [`purge`] module.
//!
//! # What this crate is not
//!
//! Baukit does not standardize a sync protocol. Wire payloads, conflict
//! resolution, batching, and the pull endpoint stay product-owned, as
//! `docs/platform/offline-readiness-contract.md` says. This crate owns
//! revision allocation, tombstone purge horizons, and timestamp generation.
//! Merge rules, retention periods, table lists, and scheduling stay
//! product-owned.
//!
//! # Schema
//!
//! Products copy [`POSTGRES_MIGRATION_SQL`] and
//! [`POSTGRES_PURGE_HORIZONS_MIGRATION_SQL`] into their own ordered
//! migrations; the crate never runs migrations at process startup. Products
//! whose existing counter uses `user_id` can instead copy
//! [`POSTGRES_RENAME_USER_ID_TO_OWNER_ID_SQL`]. The reference migration also
//! documents the column convention every syncable table follows: `id`,
//! `owner_id`, `updated_at`, `deleted_at`, `revision`, plus an `(owner_id,
//! revision)` index.
//!
//! # Usage
//!
//! Call [`next_revision`] inside the transaction that writes the row, and stamp
//! the returned value onto that row. If the transaction rolls back, the
//! allocation rolls back with it and the revision is never handed out.
//!
//! ```no_run
//! # #[cfg(feature = "sqlx-postgres")]
//! # async fn example(pool: &sqlx::PgPool, owner_id: uuid::Uuid) -> Result<(), sqlx::Error> {
//! let mut transaction = pool.begin().await?;
//! let revision = baukit_sync::next_revision(&mut transaction, owner_id).await?;
//! sqlx::query("UPDATE product_records SET revision = $1 WHERE owner_id = $2")
//!     .bind(revision)
//!     .bind(owner_id)
//!     .execute(&mut *transaction)
//!     .await?;
//! transaction.commit().await?;
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

pub mod hlc;
pub mod horizon;
#[cfg(feature = "sqlx-postgres")]
pub mod purge;
#[cfg(feature = "sqlx-postgres")]
mod revision;

#[cfg(feature = "sqlx-postgres")]
pub use revision::{current_revision, current_revision_for_update, ensure_owner, next_revision};

/// Reference PostgreSQL schema and column convention for product migrations.
///
/// Copy this SQL into a product migration; do not execute it dynamically during
/// application startup.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../migrations/0001_baukit_sync.sql");

/// Reference PostgreSQL schema for per-owner tombstone purge horizons.
///
/// Copy this SQL into a product migration after [`POSTGRES_MIGRATION_SQL`].
/// The [`purge`] module reads and writes the `sync_purge_horizons` table it
/// creates. The table name is fixed, like `sync_revisions`.
pub const POSTGRES_PURGE_HORIZONS_MIGRATION_SQL: &str =
    include_str!("../migrations/0002_baukit_sync_purge_horizons.sql");

/// One-shot PostgreSQL migration from `sync_revisions.user_id` to `owner_id`.
///
/// Copy this SQL into a product migration only when its existing table has the
/// old `user_id` shape and the foreign key has the conventional
/// `sync_revisions_user_id_fkey` name. PostgreSQL keeps the foreign key and its
/// delete action when the column is renamed. The SQL renames that constraint
/// for clarity and adds the canonical non-negative revision check.
///
/// This migration is intentionally one-shot. It fails if `owner_id` already
/// exists, which prevents a partially applicable migration from being hidden.
pub const POSTGRES_RENAME_USER_ID_TO_OWNER_ID_SQL: &str =
    include_str!("../postgres_rename_user_id_to_owner_id.sql");

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(all(doctest, feature = "sqlx-postgres"))]
struct ReadmeDoctests;
