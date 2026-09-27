//! Batched tombstone purge with per-owner horizons, and the matching pull guard.
//!
//! [`purge_tombstone_batch`] deletes at most one batch of tombstones older than
//! a caller cutoff and raises each affected owner's horizon in the caller's
//! transaction. [`purge_tombstones`] drains a caller-ordered table list, one
//! committed batch per transaction. [`guard_pull_cursor`] rejects a pull
//! cursor below the owner's horizon with
//! [`PullCursorError::ResyncRequired`].
//!
//! # Locking
//!
//! A purge batch locks its candidate rows with the caller's
//! `FOR UPDATE SKIP LOCKED`, then takes `FOR UPDATE SKIP LOCKED` on each
//! affected owner's `sync_revisions` row. It never waits for a lock. An owner
//! whose counter is locked by a writer, a pull, or another purge is skipped
//! and picked up by a later batch. [`guard_pull_cursor`] takes `FOR KEY SHARE`
//! on the same counter row, which does not block revision allocation but keeps
//! a purge for that owner out until the pull transaction ends. A pull that
//! reads its rows in that transaction therefore sees either every tombstone
//! above its cursor or a horizon that rejects the cursor.
//!
//! Tombstones whose owner has no `sync_revisions` row are never purged.

use std::{
    collections::{BTreeMap, BTreeSet},
    num::NonZeroU32,
};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Row as _, Transaction};
use thiserror::Error;
use uuid::Uuid;

use crate::horizon::{FULL_RESYNC_CURSOR, PullCursorError, check_pull_cursor};

/// Caller-owned SQL that selects and deletes one syncable table's tombstones.
///
/// `select` binds the cutoff as `$1` (`timestamptz`) and the batch limit as
/// `$2` (`bigint`). It returns at most `$2` rows with the columns `id`
/// (`uuid`), `owner_id` (`uuid`), and `revision` (`bigint`) for tombstones
/// whose `deleted_at` is before `$1`, and it must lock them with
/// `FOR UPDATE SKIP LOCKED`. A parent table's selector must exclude rows that
/// still have children, so that no cascade removes a tombstone the horizon did
/// not count.
///
/// `delete` binds the selected ids as `$1` (`uuid[]`) and deletes those rows.
/// Remove non-synced dependents with a foreign-key cascade or a data-modifying
/// `WITH` clause in the same statement.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TombstoneTable {
    name: &'static str,
    select: &'static str,
    delete: &'static str,
}

impl TombstoneTable {
    /// Creates a table entry from its report name and SQL.
    #[must_use]
    pub const fn new(name: &'static str, select: &'static str, delete: &'static str) -> Self {
        Self {
            name,
            select,
            delete,
        }
    }

    /// Returns the name used in [`PurgedTable`] reports.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }
}

/// Outcome of one purge transaction for one table.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PurgeBatch {
    /// Rows the table's selector returned and locked.
    pub selected: usize,
    /// Rows the table's delete statement removed.
    pub deleted: u64,
}

impl PurgeBatch {
    /// Returns whether another batch may find more tombstones.
    ///
    /// A full batch that deleted at least one row may have more behind it. A
    /// short batch, or one where every owner was busy, ends the drain.
    #[must_use]
    pub fn may_have_more(self, limit: NonZeroU32) -> bool {
        self.deleted > 0 && self.selected >= limit_as_usize(limit)
    }
}

/// Rows removed from one table by [`purge_tombstones`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PurgedTable {
    /// The table's [`TombstoneTable::name`].
    pub table: &'static str,
    /// Rows deleted across every batch.
    pub deleted: u64,
}

/// Failure while purging tombstones.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PurgeError {
    /// The table's selector returned more rows than the batch limit.
    #[error(
        "tombstone selector for {table} returned {selected} rows above the batch limit {limit}"
    )]
    BatchLimitExceeded {
        /// The table's [`TombstoneTable::name`].
        table: &'static str,
        /// Rows the selector returned.
        selected: usize,
        /// The batch limit it was given.
        limit: NonZeroU32,
    },
    /// A database statement failed. The batch's transaction must roll back.
    #[error("tombstone purge database operation failed")]
    Database(#[from] sqlx::Error),
}

/// Failure while checking a pull cursor against the owner's horizon.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PullGuardError {
    /// The cursor is negative or below the owner's purge horizon.
    #[error(transparent)]
    Cursor(#[from] PullCursorError),
    /// A database statement failed.
    #[error("pull guard database operation failed")]
    Database(#[from] sqlx::Error),
}

struct Candidate {
    id: Uuid,
    owner_id: Uuid,
    revision: i64,
}

/// Purges at most `limit` tombstones of one table in the caller's transaction.
///
/// The batch deletes only rows whose owner counter it could lock, and raises
/// each such owner's horizon to the greatest revision it removed. A horizon is
/// never lowered. Commit the transaction to make the deletion and the horizon
/// visible together.
///
/// # Errors
///
/// Returns [`PurgeError::BatchLimitExceeded`] when the selector ignores its
/// limit, and [`PurgeError::Database`] for any database error.
pub async fn purge_tombstone_batch(
    transaction: &mut Transaction<'_, Postgres>,
    table: &TombstoneTable,
    cutoff: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<PurgeBatch, PurgeError> {
    let candidates = select_candidates(transaction, table, cutoff, limit).await?;
    let selected = candidates.len();
    let locked_owners = lock_owner_counters(transaction, &candidates).await?;
    let purged = candidates
        .into_iter()
        .filter(|candidate| locked_owners.contains(&candidate.owner_id))
        .collect::<Vec<_>>();
    if purged.is_empty() {
        return Ok(PurgeBatch {
            selected,
            deleted: 0,
        });
    }
    let ids = purged
        .iter()
        .map(|candidate| candidate.id)
        .collect::<Vec<_>>();
    let deleted = sqlx::query(table.delete)
        .bind(&ids)
        .execute(&mut **transaction)
        .await?
        .rows_affected();
    raise_horizons(transaction, &purged).await?;
    Ok(PurgeBatch { selected, deleted })
}

/// Drains each table in order, committing one bounded batch per transaction.
///
/// The slice order is the delete order: put children before parents. A table
/// is finished when a batch comes back short or deletes nothing.
///
/// # Errors
///
/// Returns the first [`PurgeError`]. Batches committed before it stay
/// committed.
pub async fn purge_tombstones(
    pool: &PgPool,
    tables: &[TombstoneTable],
    cutoff: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<Vec<PurgedTable>, PurgeError> {
    let mut report = Vec::with_capacity(tables.len());
    for table in tables {
        let deleted = drain_table(pool, table, cutoff, limit).await?;
        report.push(PurgedTable {
            table: table.name,
            deleted,
        });
    }
    Ok(report)
}

/// Reads the owner's purge horizon. `None` means nothing was purged.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn purge_horizon(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar("SELECT horizon_revision FROM sync_purge_horizons WHERE owner_id = $1")
        .bind(owner_id)
        .fetch_optional(&mut **transaction)
        .await
}

/// Rejects a pull cursor below the owner's purge horizon.
///
/// Call this in the pull's transaction before reading any row. It holds
/// `FOR KEY SHARE` on the owner's counter row until that transaction ends, so
/// no purge for the owner commits between this check and the row reads.
///
/// # Errors
///
/// Returns [`PullGuardError::Cursor`] for a negative or stale cursor and
/// [`PullGuardError::Database`] for any database error.
pub async fn guard_pull_cursor(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    cursor: i64,
) -> Result<(), PullGuardError> {
    check_pull_cursor(cursor, FULL_RESYNC_CURSOR)?;
    sqlx::query("SELECT 1 FROM sync_revisions WHERE owner_id = $1 FOR KEY SHARE")
        .bind(owner_id)
        .execute(&mut **transaction)
        .await?;
    let horizon = purge_horizon(transaction, owner_id)
        .await?
        .unwrap_or(FULL_RESYNC_CURSOR);
    check_pull_cursor(cursor, horizon)?;
    Ok(())
}

async fn drain_table(
    pool: &PgPool,
    table: &TombstoneTable,
    cutoff: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<u64, PurgeError> {
    let mut deleted = 0_u64;
    loop {
        let mut transaction = pool.begin().await?;
        let batch = purge_tombstone_batch(&mut transaction, table, cutoff, limit).await?;
        transaction.commit().await?;
        deleted = deleted.saturating_add(batch.deleted);
        if !batch.may_have_more(limit) {
            return Ok(deleted);
        }
    }
}

async fn select_candidates(
    transaction: &mut Transaction<'_, Postgres>,
    table: &TombstoneTable,
    cutoff: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<Vec<Candidate>, PurgeError> {
    let rows = sqlx::query(table.select)
        .bind(cutoff)
        .bind(i64::from(limit.get()))
        .fetch_all(&mut **transaction)
        .await?;
    if rows.len() > limit_as_usize(limit) {
        return Err(PurgeError::BatchLimitExceeded {
            table: table.name,
            selected: rows.len(),
            limit,
        });
    }
    let mut candidates = Vec::with_capacity(rows.len());
    for row in rows {
        candidates.push(Candidate {
            id: row.try_get("id")?,
            owner_id: row.try_get("owner_id")?,
            revision: row.try_get("revision")?,
        });
    }
    Ok(candidates)
}

async fn lock_owner_counters(
    transaction: &mut Transaction<'_, Postgres>,
    candidates: &[Candidate],
) -> Result<BTreeSet<Uuid>, sqlx::Error> {
    if candidates.is_empty() {
        return Ok(BTreeSet::new());
    }
    let owners = candidates
        .iter()
        .map(|candidate| candidate.owner_id)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let locked: Vec<Uuid> = sqlx::query_scalar(
        "SELECT owner_id FROM sync_revisions
         WHERE owner_id = ANY($1)
         ORDER BY owner_id
         FOR UPDATE SKIP LOCKED",
    )
    .bind(&owners)
    .fetch_all(&mut **transaction)
    .await?;
    Ok(locked.into_iter().collect())
}

async fn raise_horizons(
    transaction: &mut Transaction<'_, Postgres>,
    purged: &[Candidate],
) -> Result<(), sqlx::Error> {
    let mut horizons = BTreeMap::<Uuid, i64>::new();
    for candidate in purged {
        horizons
            .entry(candidate.owner_id)
            .and_modify(|horizon| *horizon = (*horizon).max(candidate.revision))
            .or_insert(candidate.revision);
    }
    let (owners, revisions): (Vec<Uuid>, Vec<i64>) = horizons.into_iter().unzip();
    sqlx::query(
        "INSERT INTO sync_purge_horizons AS horizon (owner_id, horizon_revision)
         SELECT owner_id, horizon_revision
         FROM UNNEST($1::uuid[], $2::bigint[]) AS purged (owner_id, horizon_revision)
         ON CONFLICT (owner_id) DO UPDATE
         SET horizon_revision = EXCLUDED.horizon_revision, updated_at = now()
         WHERE horizon.horizon_revision < EXCLUDED.horizon_revision",
    )
    .bind(&owners)
    .bind(&revisions)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

fn limit_as_usize(limit: NonZeroU32) -> usize {
    usize::try_from(limit.get()).unwrap_or(usize::MAX)
}
