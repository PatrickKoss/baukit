#![cfg(test)]
//! F-06/F-07 regressions, the pull-conformance helper, and the P-11 sync state machine.
use std::{collections::BTreeMap, error::Error, num::NonZeroU32, path::PathBuf};

use baukit_sync::{
    horizon::PullCursorError,
    pull::{PullCursor, merge_pull_page, open_pull_window},
    purge::{PullGuardError, TombstoneTable, purge_tombstones},
};
use baukit_test::{
    PostgresTestContainer, PullConformanceAdapter, PullConformanceCursor, PullConformanceError,
    PullConformancePage, PulledRow, check_pull_conformance,
};
use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const TABLES: [&str; 2] = ["notes", "tags"];
const PURGE_LIMIT: NonZeroU32 = NonZeroU32::new(50).expect("fifty is not zero");
const SEEDS: u64 = 12;
const STEPS: usize = 40;

const PURGEABLE: [TombstoneTable; 2] = [
    TombstoneTable::new(
        "notes",
        "SELECT id, owner_id, revision FROM notes WHERE deleted_at < $1 ORDER BY deleted_at, id LIMIT $2 FOR UPDATE SKIP LOCKED",
        "DELETE FROM notes n USING UNNEST($1::uuid[], $2::uuid[]) AS p (id, owner_id) WHERE n.id = p.id AND n.owner_id = p.owner_id",
    ),
    TombstoneTable::new(
        "tags",
        "SELECT id, owner_id, revision FROM tags WHERE deleted_at < $1 ORDER BY deleted_at, id LIMIT $2 FOR UPDATE SKIP LOCKED",
        "DELETE FROM tags t USING UNNEST($1::uuid[], $2::uuid[]) AS p (id, owner_id) WHERE t.id = p.id AND t.owner_id = p.owner_id",
    ),
];

#[derive(Clone, Copy, Debug)]
enum Isolation {
    ReadCommitted,
    RepeatableRead,
}

#[derive(Clone, Copy, Debug)]
enum Cursor {
    Window,
    RevisionOnly,
    MaxAcrossTables,
}

#[derive(Clone, Debug)]
struct Write {
    table: &'static str,
    id: Uuid,
    value: i64,
    deleted: bool,
    old: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct Row {
    revision: i64,
    id: Uuid,
    value: i64,
    deleted: bool,
}

struct Page {
    rows: Vec<Row>,
    next: PullCursor,
    more: bool,
}

enum Pulled {
    Page(Page),
    Resync,
}

struct NotesAdapter {
    pool: PgPool,
    cursor: Cursor,
}

impl PullConformanceAdapter for NotesAdapter {
    type Error = TestError;

    async fn create_owner(&self) -> Result<Uuid, TestError> {
        create_owner(&self.pool).await
    }

    async fn write(
        &self,
        owner: Uuid,
        table: usize,
        rows: &[(Uuid, i64)],
    ) -> Result<(), TestError> {
        let writes: Vec<Write> = rows
            .iter()
            .map(|&(id, value)| Write {
                table: TABLES[table],
                id,
                value,
                deleted: false,
                old: false,
            })
            .collect();
        write(&self.pool, owner, &writes).await
    }

    async fn pull(
        &self,
        owner: Uuid,
        after: PullConformanceCursor,
        limit: u32,
    ) -> Result<PullConformancePage, TestError> {
        let after = PullCursor::after_row(after.0, after.1);
        let pulled = pull(
            &self.pool,
            owner,
            after,
            i64::from(limit),
            Isolation::ReadCommitted,
            self.cursor,
            &TABLES,
            None,
        )
        .await?;
        let Pulled::Page(page) = pulled else {
            return Err("unexpected resync".into());
        };
        Ok(PullConformancePage {
            rows: page
                .rows
                .iter()
                .map(|row| PulledRow {
                    revision: row.revision,
                    id: row.id,
                    value: row.value,
                })
                .collect(),
            next: (page.next.revision, page.next.id),
            more: page.more,
        })
    }
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn windowed_pull_passes_the_conformance_helper() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    let adapter = NotesAdapter {
        pool,
        cursor: Cursor::Window,
    };
    check_pull_conformance(&adapter).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn unsafe_cursors_fail_the_conformance_helper() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    for cursor in [Cursor::RevisionOnly, Cursor::MaxAcrossTables] {
        let adapter = NotesAdapter {
            pool: pool.clone(),
            cursor,
        };
        let result = check_pull_conformance(&adapter).await;
        assert!(
            matches!(result, Err(PullConformanceError::ReplicaMismatch { .. })),
            "{cursor:?} must lose rows: {result:?}"
        );
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn per_table_limits_do_not_skip_rows_of_the_shorter_table() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    let owner = create_owner(&pool).await?;
    for value in 0..4 {
        write(
            &pool,
            owner,
            &[new_write(TABLES[usize::try_from(value % 2)?], value)],
        )
        .await?;
    }
    let mut replica = BTreeMap::new();
    let mut cursor = PullCursor::START;
    loop {
        let Pulled::Page(page) = pull(
            &pool,
            owner,
            cursor,
            1,
            Isolation::ReadCommitted,
            Cursor::Window,
            &TABLES,
            None,
        )
        .await?
        else {
            return Err("unexpected resync".into());
        };
        assert!(page.rows.len() <= TABLES.len());
        for row in &page.rows {
            apply_row(&mut replica, row);
        }
        cursor = page.next;
        if !page.more {
            break;
        }
    }
    assert_eq!(replica.len(), 4);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn window_bounds_a_read_committed_pull_across_tables() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    let owner = create_owner(&pool).await?;
    write(&pool, owner, &[new_write("notes", 1)]).await?;
    let late = [new_write("notes", 2), new_write("tags", 3)];
    let Pulled::Page(page) = pull(
        &pool,
        owner,
        PullCursor::START,
        10,
        Isolation::ReadCommitted,
        Cursor::Window,
        &TABLES,
        Some((owner, &late)),
    )
    .await?
    else {
        return Err("unexpected resync".into());
    };
    assert_eq!(page.rows.len(), 1, "rows above hi wait for the next pull");
    assert_eq!(page.next, PullCursor::after_revision(1));
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn cursor_from_before_an_owner_erasure_requires_resync() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    let owner = create_owner(&pool).await?;
    for value in 0..3 {
        write(&pool, owner, &[new_write("notes", value)]).await?;
    }
    erase_owner(&pool, owner).await?;
    let mut transaction = pool.begin().await?;
    let stale = open_pull_window(&mut transaction, owner, PullCursor::after_revision(3)).await;
    assert!(matches!(
        stale,
        Err(PullGuardError::Cursor(PullCursorError::ResyncRequired {
            horizon_revision: 0
        }))
    ));
    Ok(())
}

/// P-11: random writes with several rows per revision, tombstones, purges,
/// paged pulls at READ COMMITTED and REPEATABLE READ with a writer committing
/// between table reads, and owner erasure. After every full pull the replica
/// equals the reference model.
#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn sync_state_machine_replica_matches_reference() -> Result<(), TestError> {
    let (_fixture, pool) = fixture().await?;
    for seed in 1..=SEEDS {
        Machine::new(&pool, seed).await?.run().await?;
    }
    Ok(())
}

struct Machine<'a> {
    pool: &'a PgPool,
    random: Random,
    owner: Uuid,
    ids: Vec<(&'static str, Uuid)>,
    reference: BTreeMap<Uuid, i64>,
    replica: BTreeMap<Uuid, i64>,
    cursor: PullCursor,
    next_value: i64,
}

impl<'a> Machine<'a> {
    async fn new(pool: &'a PgPool, seed: u64) -> Result<Self, TestError> {
        Ok(Self {
            pool,
            random: Random(seed),
            owner: create_owner(pool).await?,
            ids: Vec::new(),
            reference: BTreeMap::new(),
            replica: BTreeMap::new(),
            cursor: PullCursor::START,
            next_value: 0,
        })
    }

    async fn run(mut self) -> Result<(), TestError> {
        for _ in 0..STEPS {
            match self.random.below(10) {
                0..=4 => {
                    let writes = self.writes();
                    self.apply(&writes).await?;
                }
                5 => self.purge().await?,
                6 => self.erase().await?,
                _ => {
                    self.full_pull().await?;
                    assert_eq!(self.replica, self.reference, "seed {}", self.random.0);
                }
            }
        }
        self.full_pull().await?;
        assert_eq!(self.replica, self.reference);
        Ok(())
    }

    fn writes(&mut self) -> Vec<Write> {
        let count = 1 + self.random.below(3);
        let mut writes: Vec<Write> = Vec::new();
        for _ in 0..count {
            let reuse = !self.ids.is_empty() && self.random.below(2) == 0;
            let (table, id) = if reuse {
                self.ids[self.random.below(self.ids.len())]
            } else {
                let fresh = (TABLES[self.random.below(TABLES.len())], Uuid::now_v7());
                self.ids.push(fresh);
                fresh
            };
            if writes.iter().any(|write| write.id == id) {
                continue;
            }
            self.next_value += 1;
            let deleted = self.random.below(4) == 0;
            writes.push(Write {
                table,
                id,
                value: self.next_value,
                deleted,
                old: deleted && self.random.below(2) == 0,
            });
        }
        writes
    }

    async fn apply(&mut self, writes: &[Write]) -> Result<(), TestError> {
        write(self.pool, self.owner, writes).await?;
        record(&mut self.reference, writes);
        Ok(())
    }

    async fn purge(&self) -> Result<(), TestError> {
        let cutoff = Timestamp::now() - SignedDuration::from_hours(24);
        purge_tombstones(self.pool, &PURGEABLE, cutoff, PURGE_LIMIT).await?;
        Ok(())
    }

    async fn erase(&mut self) -> Result<(), TestError> {
        erase_rows_keeping_counter(self.pool, self.owner).await?;
        self.ids.clear();
        self.reference.clear();
        Ok(())
    }

    async fn full_pull(&mut self) -> Result<(), TestError> {
        let isolation = if self.random.below(2) == 0 {
            Isolation::ReadCommitted
        } else {
            Isolation::RepeatableRead
        };
        loop {
            let concurrent = (self.random.below(3) == 0).then(|| self.writes());
            let interleaved = concurrent.as_deref().map(|writes| (self.owner, writes));
            let limit = i64::try_from(1 + self.random.below(2))?;
            let pulled = pull(
                self.pool,
                self.owner,
                self.cursor,
                limit,
                isolation,
                Cursor::Window,
                &TABLES,
                interleaved,
            )
            .await?;
            if let Some(writes) = &concurrent {
                record(&mut self.reference, writes);
            }
            let page = match pulled {
                Pulled::Resync => {
                    self.replica.clear();
                    self.cursor = PullCursor::START;
                    continue;
                }
                Pulled::Page(page) => page,
            };
            for row in &page.rows {
                apply_row(&mut self.replica, row);
            }
            self.cursor = page.next;
            if !page.more && concurrent.is_none() {
                return Ok(());
            }
        }
    }
}

struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        usize::try_from(self.0 % u64::try_from(bound).unwrap_or(1)).unwrap_or(0)
    }
}

fn record(reference: &mut BTreeMap<Uuid, i64>, writes: &[Write]) {
    for write in writes {
        if write.deleted {
            reference.remove(&write.id);
        } else {
            reference.insert(write.id, write.value);
        }
    }
}

fn apply_row(replica: &mut BTreeMap<Uuid, i64>, row: &Row) {
    if row.deleted {
        replica.remove(&row.id);
    } else {
        replica.insert(row.id, row.value);
    }
}

fn new_write(table: &'static str, value: i64) -> Write {
    Write {
        table,
        id: Uuid::now_v7(),
        value,
        deleted: false,
        old: false,
    }
}

async fn write(pool: &PgPool, owner: Uuid, writes: &[Write]) -> Result<(), TestError> {
    let mut transaction = pool.begin().await?;
    let revision = baukit_sync::next_revision(&mut transaction, owner).await?;
    for write in writes {
        upsert(&mut transaction, owner, revision, write).await?;
    }
    transaction.commit().await?;
    Ok(())
}

async fn upsert(
    transaction: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    revision: i64,
    write: &Write,
) -> Result<(), TestError> {
    let deleted_at = write.deleted.then(|| {
        let age = if write.old { 48 } else { 0 };
        (Timestamp::now() - SignedDuration::from_hours(age)).to_sqlx()
    });
    let sql = format!(
        "INSERT INTO {table} (owner_id, id, value, deleted_at, revision) VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (owner_id, id) DO UPDATE
         SET value = EXCLUDED.value, deleted_at = EXCLUDED.deleted_at, revision = EXCLUDED.revision",
        table = write.table
    );
    sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(owner)
        .bind(write.id)
        .bind(write.value)
        .bind(deleted_at)
        .bind(revision)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

/// Serves one page across `tables`, optionally committing `interleaved`
/// writes from another connection between the first and second table read.
#[expect(clippy::too_many_arguments, reason = "test driver with explicit knobs")]
async fn pull(
    pool: &PgPool,
    owner: Uuid,
    after: PullCursor,
    limit: i64,
    isolation: Isolation,
    cursor: Cursor,
    tables: &[&'static str],
    interleaved: Option<(Uuid, &[Write])>,
) -> Result<Pulled, TestError> {
    let mut transaction = pool.begin().await?;
    if let Isolation::RepeatableRead = isolation {
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ")
            .execute(&mut *transaction)
            .await?;
    }
    let window = match open_pull_window(&mut transaction, owner, after).await {
        Ok(window) => window,
        Err(PullGuardError::Cursor(PullCursorError::ResyncRequired { .. })) => {
            return Ok(Pulled::Resync);
        }
        Err(error) => return Err(error.into()),
    };
    let mut per_table = Vec::new();
    for (index, table) in tables.iter().enumerate() {
        if index == 1
            && let Some((writer, writes)) = interleaved
        {
            write(pool, writer, writes).await?;
        }
        per_table.push(read_table(&mut transaction, owner, table, &window, limit, cursor).await?);
    }
    transaction.commit().await?;
    if tables.len() < 2
        && let Some((writer, writes)) = interleaved
    {
        write(pool, writer, writes).await?;
    }
    let limit = usize::try_from(limit)?;
    Ok(Pulled::Page(match cursor {
        Cursor::Window => {
            let page = merge_pull_page(&window, per_table, limit, |row: &Row| {
                PullCursor::after_row(row.revision, row.id)
            });
            Page {
                rows: page.rows,
                next: page.next,
                more: page.more,
            }
        }
        Cursor::RevisionOnly | Cursor::MaxAcrossTables => {
            unsafe_merge(per_table, limit, after, cursor)
        }
    }))
}

/// The buggy merges kept as negative controls: a revision-only cursor, or the
/// highest last position across tables with per-table limits.
fn unsafe_merge(per_table: Vec<Vec<Row>>, limit: usize, after: PullCursor, cursor: Cursor) -> Page {
    let more = per_table.iter().any(|rows| rows.len() >= limit);
    let mut rows: Vec<Row> = per_table.into_iter().flatten().collect();
    rows.sort_by_key(|row| (row.revision, row.id));
    let next = rows.last().map_or(after, |row| match cursor {
        Cursor::RevisionOnly => PullCursor::after_revision(row.revision),
        _ => PullCursor::after_row(row.revision, row.id),
    });
    Page { rows, next, more }
}

async fn read_table(
    transaction: &mut Transaction<'_, Postgres>,
    owner: Uuid,
    table: &'static str,
    window: &baukit_sync::pull::PullWindow,
    limit: i64,
    cursor: Cursor,
) -> Result<Vec<Row>, TestError> {
    let sql = match cursor {
        Cursor::Window | Cursor::MaxAcrossTables => format!(
            "SELECT revision, id, value, deleted_at IS NOT NULL FROM {table}
             WHERE owner_id = $1 AND revision <= $2 AND (revision, id) > ($3, $4)
             ORDER BY revision, id LIMIT $5"
        ),
        Cursor::RevisionOnly => format!(
            "SELECT revision, id, value, deleted_at IS NOT NULL FROM {table}
             WHERE owner_id = $1 AND $2 >= 0 AND revision > $3 AND $4 IS NOT NULL
             ORDER BY revision LIMIT $5"
        ),
    };
    let rows: Vec<(i64, Uuid, i64, bool)> = sqlx::query_as(sqlx::AssertSqlSafe(sql))
        .bind(owner)
        .bind(window.hi)
        .bind(window.after.revision)
        .bind(window.after.id)
        .bind(limit)
        .fetch_all(&mut **transaction)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(revision, id, value, deleted)| Row {
            revision,
            id,
            value,
            deleted,
        })
        .collect())
}

/// The erasure the README recommends: delete the rows, keep the counter, and
/// raise the horizon past every cursor a client may hold.
async fn erase_rows_keeping_counter(pool: &PgPool, owner: Uuid) -> Result<(), TestError> {
    let mut transaction = pool.begin().await?;
    for table in TABLES {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {table} WHERE owner_id = $1"
        )))
        .bind(owner)
        .execute(&mut *transaction)
        .await?;
    }
    let horizon = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query(
        "INSERT INTO sync_purge_horizons (owner_id, horizon_revision) VALUES ($1, $2)
         ON CONFLICT (owner_id) DO UPDATE SET horizon_revision = EXCLUDED.horizon_revision",
    )
    .bind(owner)
    .bind(horizon)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(())
}

async fn erase_owner(pool: &PgPool, owner: Uuid) -> Result<(), TestError> {
    sqlx::query("DELETE FROM sync_revisions WHERE owner_id = $1")
        .bind(owner)
        .execute(pool)
        .await?;
    let mut transaction = pool.begin().await?;
    baukit_sync::ensure_owner(&mut transaction, owner).await?;
    transaction.commit().await?;
    Ok(())
}

async fn create_owner(pool: &PgPool) -> Result<Uuid, TestError> {
    let owner = Uuid::new_v4();
    let mut transaction = pool.begin().await?;
    baukit_sync::ensure_owner(&mut transaction, owner).await?;
    transaction.commit().await?;
    Ok(owner)
}

async fn fixture() -> Result<(PostgresTestContainer, PgPool), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    for table in TABLES {
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TABLE {table} (
                 owner_id UUID NOT NULL REFERENCES sync_revisions (owner_id) ON DELETE CASCADE,
                 id UUID NOT NULL,
                 value BIGINT NOT NULL,
                 deleted_at TIMESTAMPTZ,
                 revision BIGINT NOT NULL,
                 PRIMARY KEY (owner_id, id)
             );
             CREATE INDEX {table}_sync_idx ON {table} (owner_id, revision, id);"
        )))
        .execute(&pool)
        .await?;
    }
    Ok((fixture, pool))
}
