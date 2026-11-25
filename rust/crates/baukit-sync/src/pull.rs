//! Snapshot-safe pull windows with a `(revision, id)` tuple cursor.
//!
//! A revision can stamp several rows, so a cursor that only holds a revision
//! loses the rest of a revision when a page ends in the middle of it. A pull
//! also runs one statement per table, and under READ COMMITTED each statement
//! sees writes that committed after the previous one. [`open_pull_window`]
//! fixes both: it reads the owner's committed counter first and returns it as
//! the window's upper bound `hi`, and every table query pages with
//!
//! ```sql
//! WHERE owner_id = $1 AND revision <= $2 AND (revision, id) > ($3, $4)
//! ORDER BY revision, id
//! LIMIT $5
//! ```
//!
//! binding [`PullWindow::hi`], `after.revision` and `after.id`. Revision
//! allocation serializes on the counter row, so every write stamped at or
//! below a committed counter value has itself committed: rows up to `hi` are
//! visible to every later statement at either isolation level, and rows above
//! it wait for the next pull.
//!
//! Each table query has its own `LIMIT`, so tables stop at different
//! positions. Storing the highest position across tables skips the rows a
//! table cut off below it. [`merge_pull_page`] builds one page instead: it
//! cuts every table at the lowest last position among tables that filled
//! their limit and reports `more`.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::{
    horizon::{FULL_RESYNC_CURSOR, PullCursorError, check_pull_cursor},
    purge::{PullGuardError, purge_horizon},
};

/// Position after the last row a client applied.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PullCursor {
    /// Revision of the last applied row.
    pub revision: i64,
    /// Id of the last applied row within `revision`.
    pub id: Uuid,
}

impl PullCursor {
    /// The cursor before every row, for a first or full pull.
    pub const START: Self = Self::after_revision(FULL_RESYNC_CURSOR);

    /// A cursor past every row stamped at or below `revision`.
    ///
    /// Converts a legacy revision-only cursor.
    #[must_use]
    pub const fn after_revision(revision: i64) -> Self {
        Self {
            revision,
            id: Uuid::max(),
        }
    }

    /// The cursor after `(revision, id)`, the last row of a page.
    #[must_use]
    pub const fn after_row(revision: i64, id: Uuid) -> Self {
        Self { revision, id }
    }
}

/// Bounds of one pull, returned by [`open_pull_window`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PullWindow {
    /// The owner's committed revision counter when the window opened.
    pub hi: i64,
    /// Exclusive lower bound of the rows this pull returns.
    pub after: PullCursor,
}

impl PullWindow {
    /// The cursor a client stores once every table returned a short page.
    #[must_use]
    pub const fn drained_cursor(&self) -> PullCursor {
        PullCursor::after_revision(self.hi)
    }
}

/// One merged pull page from [`merge_pull_page`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergedPullPage<T> {
    /// Rows from every table in `(revision, id)` order.
    pub rows: Vec<T>,
    /// Cursor the client stores after applying `rows`.
    pub next: PullCursor,
    /// Whether rows remain at or below `hi`; the client pulls again.
    pub more: bool,
}

/// Merges per-table pages read with the same `limit` into one page.
///
/// Each table's rows must be in `(revision, id)` order, as the predicate in
/// the [module docs](self) returns them. A table that returned `limit` rows
/// may have more, so the page ends at the lowest last position among those
/// tables, rows above it are dropped, and `more` is set. When no table filled
/// its limit the page drains the window.
#[must_use]
pub fn merge_pull_page<T>(
    window: &PullWindow,
    tables: Vec<Vec<T>>,
    limit: usize,
    position: impl Fn(&T) -> PullCursor,
) -> MergedPullPage<T> {
    let cut = tables
        .iter()
        .filter(|rows| rows.len() >= limit)
        .filter_map(|rows| rows.last().map(&position))
        .min();
    let mut rows: Vec<T> = tables
        .into_iter()
        .flatten()
        .filter(|row| cut.is_none_or(|cut| position(row) <= cut))
        .collect();
    rows.sort_by_key(|row| position(row));
    match cut {
        Some(next) => MergedPullPage {
            rows,
            next,
            more: true,
        },
        None => MergedPullPage {
            rows,
            next: window.drained_cursor(),
            more: false,
        },
    }
}

/// Opens a pull window: checks the cursor against the purge horizon and reads
/// the owner's high-water mark.
///
/// Call it first in the pull transaction and read every table in that same
/// transaction with the predicate from the [module docs](self). It takes
/// `FOR KEY SHARE` on the owner's counter row, which does not block
/// `next_revision` but keeps a purge out until the transaction ends. An owner
/// without a counter row has `hi = 0`.
///
/// A cursor above `hi` can only come from before the owner's counter was
/// erased and recreated. It gets `resync_required` instead of an empty page,
/// which would otherwise leave the client stuck behind a counter that restarted
/// at zero.
///
/// # Errors
///
/// Returns [`PullGuardError::Cursor`] for a negative, stale, or reset cursor and
/// [`PullGuardError::Database`] for any database error.
pub async fn open_pull_window(
    transaction: &mut Transaction<'_, Postgres>,
    owner_id: Uuid,
    after: PullCursor,
) -> Result<PullWindow, PullGuardError> {
    check_pull_cursor(after.revision, FULL_RESYNC_CURSOR)?;
    let hi: Option<i64> = sqlx::query_scalar(
        "SELECT last_revision FROM sync_revisions WHERE owner_id = $1 FOR KEY SHARE",
    )
    .bind(owner_id)
    .fetch_optional(&mut **transaction)
    .await?;
    let horizon = purge_horizon(transaction, owner_id)
        .await?
        .unwrap_or(FULL_RESYNC_CURSOR);
    check_pull_cursor(after.revision, horizon)?;
    let hi = hi.unwrap_or(FULL_RESYNC_CURSOR);
    if after.revision > hi {
        return Err(PullCursorError::ResyncRequired {
            horizon_revision: hi,
        }
        .into());
    }
    Ok(PullWindow { hi, after })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_cursor_sorts_after_every_row_of_its_revision() {
        let row = PullCursor::after_row(7, Uuid::now_v7());
        assert!(PullCursor::after_revision(7) > row);
        assert!(PullCursor::after_revision(6) < row);
        assert!(PullCursor::START < PullCursor::after_row(1, Uuid::nil()));
    }

    #[test]
    fn merged_page_stops_at_the_lowest_full_table() {
        let window = PullWindow {
            hi: 9,
            after: PullCursor::START,
        };
        let id = Uuid::nil();
        let notes = vec![(8, id), (9, id)];
        let tags = vec![(4, id), (5, id)];
        let page = merge_pull_page(&window, vec![notes, tags], 2, |&(revision, id)| {
            PullCursor::after_row(revision, id)
        });
        assert_eq!(page.rows, vec![(4, id), (5, id)]);
        assert_eq!(page.next, PullCursor::after_row(5, id));
        assert!(page.more);
    }

    #[test]
    fn merged_page_drains_the_window_when_no_table_is_full() {
        let window = PullWindow {
            hi: 9,
            after: PullCursor::START,
        };
        let page = merge_pull_page(
            &window,
            vec![vec![(3, Uuid::nil())], Vec::new()],
            2,
            |&(r, id)| PullCursor::after_row(r, id),
        );
        assert_eq!(page.next, PullCursor::after_revision(9));
        assert!(!page.more);
    }
}
