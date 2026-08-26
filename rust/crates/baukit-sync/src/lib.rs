//! Per-owner revision allocation for incremental sync.
//!
//! A syncable row carries a `revision` drawn from a counter that is private to
//! its owner. A client pulls by asking for everything above the revision it
//! last saw, so the counter must be monotonic per owner and must move in the
//! same transaction as the row write it stamps. [`next_revision`] does exactly
//! that and nothing else.
//!
//! # What this crate is not
//!
//! Baukit does not standardize a sync protocol. Wire payloads, conflict
//! resolution, batching, and the pull endpoint stay product-owned, as
//! `docs/platform/offline-readiness-contract.md` says. This crate owns one
//! mechanism: allocating the next revision safely.
//!
//! # Schema
//!
//! Products copy [`POSTGRES_MIGRATION_SQL`] into their own ordered migrations;
//! the crate never runs migrations at process startup. Products whose existing
//! counter uses `user_id` can instead copy
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

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Reference PostgreSQL schema and column convention for product migrations.
///
/// Copy this SQL into a product migration; do not execute it dynamically during
/// application startup.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../migrations/0001_baukit_sync.sql");

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

/// Allocates the owner's next revision inside the caller's transaction.
///
/// The `UPDATE ... RETURNING` takes a row lock for the duration of the
/// transaction, so concurrent writers for one owner serialize and each sees a
/// distinct, increasing value. Different owners touch different rows and do not
/// block each other. A rollback discards the allocation.
///
/// The owner's counter row must exist; call [`ensure_owner`] once when the
/// owner is created.
///
/// # Errors
///
/// Returns [`sqlx::Error::RowNotFound`] when the owner has no counter row, and
/// any other database error unchanged.
pub async fn next_revision(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(
        "UPDATE sync_revisions
         SET last_revision = last_revision + 1
         WHERE owner_id = $1
         RETURNING last_revision",
    )
    .bind(owner_id)
    .fetch_one(&mut **transaction)
    .await
}

/// Creates the owner's revision counter if it does not exist yet.
///
/// Call this when the owner is created, in the same transaction. Calling it
/// again is harmless and never resets an existing counter.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn ensure_owner(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO sync_revisions (owner_id)
         VALUES ($1)
         ON CONFLICT (owner_id) DO NOTHING",
    )
    .bind(owner_id)
    .execute(&mut **transaction)
    .await
    .map(|_| ())
}

/// Reads the owner's current revision without allocating a new one.
///
/// Use this to answer "what is the newest revision a pull could return". It
/// takes no lock, so a concurrent writer may advance the counter immediately
/// after the read.
///
/// # Errors
///
/// Returns any database error unchanged. An owner without a counter row reads
/// as `None`.
pub async fn current_revision(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT last_revision FROM sync_revisions WHERE owner_id = $1")
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await
}

/// Reads and locks the owner's current revision without allocating a new one.
///
/// The row lock lasts until the caller's transaction commits or rolls back.
/// Use this before a read-dependent write that must not race another revision
/// allocation. [`current_revision`] remains the non-locking read for pull
/// boundaries and status queries.
///
/// # Errors
///
/// Returns any database error unchanged. An owner without a counter row reads
/// as `None` and no row is locked.
pub async fn current_revision_for_update(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT last_revision FROM sync_revisions WHERE owner_id = $1 FOR UPDATE")
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await
}

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
