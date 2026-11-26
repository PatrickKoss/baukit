//! Pull-pagination conformance for product sync endpoints.
//!
//! The pull SQL is product-owned. This check drives writes that stamp several
//! rows with one revision across two tables with interleaved revisions,
//! overwrites rows while a pull is paging, pages with a per-table limit of 1,
//! and asserts that the client replica equals the source of truth.

use std::{collections::BTreeMap, fmt, future::Future};

use uuid::Uuid;

const ROWS_PER_REVISION: usize = 3;
const SEED_REVISIONS: usize = 3;
const CONCURRENT_WRITES: usize = 6;
const MAX_PAGES: usize = 10_000;
const PAGE_SIZE: u32 = 1;
/// Number of tables the adapter must serve; `write` gets `0..PULL_CONFORMANCE_TABLES`.
pub const PULL_CONFORMANCE_TABLES: usize = 2;

/// `(revision, id)` position after the last applied row.
pub type PullConformanceCursor = (i64, Uuid);

/// One row a pull returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PulledRow {
    /// Revision stamped on the row.
    pub revision: i64,
    /// Row id.
    pub id: Uuid,
    /// The row's value; the harness writes distinct values.
    pub value: i64,
}

/// One served pull page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullConformancePage {
    /// Rows in `(revision, id)` order.
    pub rows: Vec<PulledRow>,
    /// Cursor the client stores after applying `rows`.
    pub next: PullConformanceCursor,
    /// Whether the client should pull again right away.
    pub more: bool,
}

/// Product adapter exercised by [`check_pull_conformance`].
pub trait PullConformanceAdapter {
    /// Product failure. Its message never appears in harness output.
    type Error;

    /// Creates an owner with a revision counter and no rows.
    fn create_owner(&self) -> impl Future<Output = Result<Uuid, Self::Error>>;

    /// Upserts every `(id, value)` into `table` in one transaction stamped
    /// with one revision.
    fn write(
        &self,
        owner: Uuid,
        table: usize,
        rows: &[(Uuid, i64)],
    ) -> impl Future<Output = Result<(), Self::Error>>;

    /// Serves one merged pull page after `after`, reading every table with
    /// `LIMIT limit`.
    fn pull(
        &self,
        owner: Uuid,
        after: PullConformanceCursor,
        limit: u32,
    ) -> impl Future<Output = Result<PullConformancePage, Self::Error>>;
}

/// A pull-conformance violation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PullConformanceError {
    /// The adapter failed at the named step.
    Adapter(&'static str),
    /// A page broke `(revision, id)` order or exceeded `limit` rows per table.
    PageOrder,
    /// Paging never finished.
    Unbounded,
    /// The replica differs from the source after a full pull.
    ReplicaMismatch {
        /// Rows whose replica value is missing or stale.
        differing_rows: usize,
    },
}

impl fmt::Display for PullConformanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Adapter(step) => write!(formatter, "pull adapter failed during {step}"),
            Self::PageOrder => formatter.write_str("pull page is out of (revision, id) order"),
            Self::Unbounded => formatter.write_str("pull paging did not finish"),
            Self::ReplicaMismatch { differing_rows } => write!(
                formatter,
                "replica differs from the source in {differing_rows} rows"
            ),
        }
    }
}

impl std::error::Error for PullConformanceError {}

/// Panics with the violation when [`check_pull_conformance`] fails.
pub async fn assert_pull_conformance<A: PullConformanceAdapter>(adapter: &A) {
    if let Err(error) = check_pull_conformance(adapter).await {
        panic!("pull conformance failed: {error}");
    }
}

/// Runs the pull-pagination conformance check.
///
/// # Errors
///
/// Returns the first [`PullConformanceError`] found.
pub async fn check_pull_conformance<A: PullConformanceAdapter>(
    adapter: &A,
) -> Result<(), PullConformanceError> {
    let owner = adapter
        .create_owner()
        .await
        .map_err(|_| PullConformanceError::Adapter("create_owner"))?;
    let ids: Vec<Uuid> = (0..ROWS_PER_REVISION * SEED_REVISIONS)
        .map(|_| Uuid::now_v7())
        .collect();
    let mut source = Source::default();
    for chunk in ids.chunks(ROWS_PER_REVISION) {
        source.write(adapter, owner, chunk).await?;
    }
    for id in (0..2 * PULL_CONFORMANCE_TABLES).map(|_| Uuid::now_v7()) {
        source.write(adapter, owner, &[id]).await?;
    }
    let mut replica = BTreeMap::new();
    let pulled = pull_all(adapter, owner, &mut replica, (0, Uuid::nil()));
    let written = write_while_paging(adapter, owner, &ids, &mut source);
    let (pulled, written) = tokio::join!(pulled, written);
    written?;
    let cursor = pulled?;
    pull_all(adapter, owner, &mut replica, cursor).await?;
    source.compare(&replica)
}

#[derive(Default)]
struct Source {
    rows: BTreeMap<Uuid, i64>,
    tables: BTreeMap<Uuid, usize>,
    next_value: i64,
    writes: usize,
}

impl Source {
    async fn write<A: PullConformanceAdapter>(
        &mut self,
        adapter: &A,
        owner: Uuid,
        ids: &[Uuid],
    ) -> Result<(), PullConformanceError> {
        let fresh_table = self.writes % PULL_CONFORMANCE_TABLES;
        self.writes += 1;
        let mut by_table: BTreeMap<usize, Vec<(Uuid, i64)>> = BTreeMap::new();
        for id in ids {
            self.next_value += 1;
            let table = *self.tables.entry(*id).or_insert(fresh_table);
            by_table
                .entry(table)
                .or_default()
                .push((*id, self.next_value));
        }
        for (table, batch) in by_table {
            adapter
                .write(owner, table, &batch)
                .await
                .map_err(|_| PullConformanceError::Adapter("write"))?;
            self.rows.extend(batch);
        }
        Ok(())
    }

    fn compare(&self, replica: &BTreeMap<Uuid, i64>) -> Result<(), PullConformanceError> {
        let differing_rows = self
            .rows
            .iter()
            .filter(|(id, value)| replica.get(*id) != Some(*value))
            .count();
        if differing_rows == 0 && replica.len() == self.rows.len() {
            return Ok(());
        }
        Err(PullConformanceError::ReplicaMismatch {
            differing_rows: differing_rows.max(1),
        })
    }
}

async fn write_while_paging<A: PullConformanceAdapter>(
    adapter: &A,
    owner: Uuid,
    ids: &[Uuid],
    source: &mut Source,
) -> Result<(), PullConformanceError> {
    for round in 0..CONCURRENT_WRITES {
        tokio::task::yield_now().await;
        let mut rewritten: Vec<Uuid> = ids
            .iter()
            .cycle()
            .skip(round * 2)
            .take(2)
            .copied()
            .collect();
        rewritten.push(Uuid::now_v7());
        source.write(adapter, owner, &rewritten).await?;
    }
    Ok(())
}

async fn pull_all<A: PullConformanceAdapter>(
    adapter: &A,
    owner: Uuid,
    replica: &mut BTreeMap<Uuid, i64>,
    mut cursor: PullConformanceCursor,
) -> Result<PullConformanceCursor, PullConformanceError> {
    for _ in 0..MAX_PAGES {
        let page = adapter
            .pull(owner, cursor, PAGE_SIZE)
            .await
            .map_err(|_| PullConformanceError::Adapter("pull"))?;
        check_page(&page, cursor)?;
        replica.extend(page.rows.iter().map(|row| (row.id, row.value)));
        cursor = page.next;
        if !page.more {
            return Ok(cursor);
        }
        tokio::task::yield_now().await;
    }
    Err(PullConformanceError::Unbounded)
}

fn check_page(
    page: &PullConformancePage,
    after: PullConformanceCursor,
) -> Result<(), PullConformanceError> {
    let per_table = usize::try_from(PAGE_SIZE).unwrap_or(usize::MAX);
    let too_long = page.rows.len() > per_table.saturating_mul(PULL_CONFORMANCE_TABLES);
    let mut previous = after;
    for row in &page.rows {
        let position = (row.revision, row.id);
        if position <= previous {
            return Err(PullConformanceError::PageOrder);
        }
        previous = position;
    }
    if too_long || page.next < previous {
        return Err(PullConformanceError::PageOrder);
    }
    Ok(())
}
