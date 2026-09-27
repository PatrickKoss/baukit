use std::{error::Error, num::NonZeroU32, path::PathBuf, time::Duration};

use baukit_sync::{
    horizon::{PullCursorError, check_pull_cursor},
    purge::{
        PullGuardError, PurgeError, PurgedTable, TombstoneTable, guard_pull_cursor, purge_horizon,
        purge_tombstone_batch, purge_tombstones,
    },
};
use baukit_test::{
    PostgresTestContainer, PullPause, PurgeHorizonAdapter, PurgeHorizonPull,
    assert_purge_horizon_conformance, check_purge_horizon_conformance,
};
use chrono::{DateTime, TimeDelta, Utc};
use sqlx::PgPool;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const LIMIT: NonZeroU32 = NonZeroU32::new(2).expect("two is not zero");
const LOCK_WAIT: Duration = Duration::from_secs(5);

const RECORDS: TombstoneTable = TombstoneTable::new(
    "product_records",
    "SELECT id, owner_id, revision FROM product_records
     WHERE deleted_at < $1
     ORDER BY deleted_at, id
     LIMIT $2
     FOR UPDATE SKIP LOCKED",
    "DELETE FROM product_records WHERE id = ANY($1)",
);

const ITEMS: TombstoneTable = TombstoneTable::new(
    "product_items",
    "SELECT item.id, list.owner_id, item.revision
     FROM product_items item
     JOIN product_lists list ON list.id = item.list_id
     WHERE item.deleted_at < $1
     ORDER BY item.deleted_at, item.id
     LIMIT $2
     FOR UPDATE OF item SKIP LOCKED",
    "DELETE FROM product_items WHERE id = ANY($1)",
);

const LISTS: TombstoneTable = TombstoneTable::new(
    "product_lists",
    "SELECT list.id, list.owner_id, list.revision
     FROM product_lists list
     WHERE list.deleted_at < $1
       AND NOT EXISTS (SELECT 1 FROM product_items item WHERE item.list_id = list.id)
     ORDER BY list.deleted_at, list.id
     LIMIT $2
     FOR UPDATE SKIP LOCKED",
    "DELETE FROM product_lists WHERE id = ANY($1)",
);

#[derive(Clone, Copy)]
enum Guard {
    InPullTransaction,
    BeforePullTransaction,
}

struct Adapter {
    pool: PgPool,
    guard: Guard,
}

impl PurgeHorizonAdapter for Adapter {
    type Owner = Uuid;
    type Error = TestError;

    async fn create_owner(&self) -> Result<Uuid, TestError> {
        owner(&self.pool).await
    }

    async fn write_live_row(&self, owner: &Uuid) -> Result<i64, TestError> {
        let mut transaction = self.pool.begin().await?;
        let revision = baukit_sync::next_revision(&mut transaction, *owner).await?;
        sqlx::query("INSERT INTO product_records (id, owner_id, revision) VALUES ($1, $2, $3)")
            .bind(Uuid::new_v4())
            .bind(owner)
            .bind(revision)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(revision)
    }

    async fn write_tombstone(
        &self,
        owner: &Uuid,
        deleted_at: DateTime<Utc>,
    ) -> Result<i64, TestError> {
        tombstone_record(&self.pool, *owner, deleted_at).await
    }

    async fn purge_batch(
        &self,
        cutoff: DateTime<Utc>,
        limit: NonZeroU32,
    ) -> Result<u64, TestError> {
        let mut transaction = self.pool.begin().await?;
        let batch = purge_tombstone_batch(&mut transaction, &RECORDS, cutoff, limit).await?;
        transaction.commit().await?;
        Ok(batch.deleted)
    }

    async fn pull(
        &self,
        owner: &Uuid,
        cursor: i64,
        pause: PullPause,
    ) -> Result<PurgeHorizonPull, TestError> {
        match self.guard {
            Guard::InPullTransaction => {
                pull_in_one_transaction(&self.pool, *owner, cursor, pause).await
            }
            Guard::BeforePullTransaction => {
                pull_after_separate_guard(&self.pool, *owner, cursor, pause).await
            }
        }
    }

    async fn purge_horizon(&self, owner: &Uuid) -> Result<Option<i64>, TestError> {
        let mut transaction = self.pool.begin().await?;
        let horizon = purge_horizon(&mut transaction, *owner).await?;
        transaction.commit().await?;
        Ok(horizon)
    }

    async fn erase_owner(&self, owner: &Uuid) -> Result<(), TestError> {
        sqlx::query("DELETE FROM sync_revisions WHERE owner_id = $1")
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

async fn pull_in_one_transaction(
    pool: &PgPool,
    owner: Uuid,
    cursor: i64,
    pause: PullPause,
) -> Result<PurgeHorizonPull, TestError> {
    let mut transaction = pool.begin().await?;
    match guard_pull_cursor(&mut transaction, owner, cursor).await {
        Ok(()) => {}
        Err(PullGuardError::Cursor(PullCursorError::ResyncRequired { horizon_revision })) => {
            return Ok(PurgeHorizonPull::ResyncRequired { horizon_revision });
        }
        Err(error) => return Err(error.into()),
    }
    pause.reached().await;
    let revisions = sqlx::query_scalar(
        "SELECT revision FROM product_records
         WHERE owner_id = $1 AND revision > $2
         ORDER BY revision",
    )
    .bind(owner)
    .bind(cursor)
    .fetch_all(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(PurgeHorizonPull::Page { revisions })
}

async fn pull_after_separate_guard(
    pool: &PgPool,
    owner: Uuid,
    cursor: i64,
    pause: PullPause,
) -> Result<PurgeHorizonPull, TestError> {
    let horizon: Option<i64> =
        sqlx::query_scalar("SELECT horizon_revision FROM sync_purge_horizons WHERE owner_id = $1")
            .bind(owner)
            .fetch_optional(pool)
            .await?;
    if let Err(PullCursorError::ResyncRequired { horizon_revision }) =
        check_pull_cursor(cursor, horizon.unwrap_or_default())
    {
        return Ok(PurgeHorizonPull::ResyncRequired { horizon_revision });
    }
    pause.reached().await;
    let revisions = sqlx::query_scalar(
        "SELECT revision FROM product_records
         WHERE owner_id = $1 AND revision > $2
         ORDER BY revision",
    )
    .bind(owner)
    .bind(cursor)
    .fetch_all(pool)
    .await?;
    Ok(PurgeHorizonPull::Page { revisions })
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_purge_helpers_pass_the_purge_horizon_conformance() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let adapter = Adapter {
        pool: pool.clone(),
        guard: Guard::InPullTransaction,
    };

    assert_purge_horizon_conformance(&adapter).await;

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_horizon_read_outside_the_pull_transaction_fails_the_race_case() -> Result<(), TestError>
{
    let (fixture, pool) = fixture().await?;
    let adapter = Adapter {
        pool: pool.clone(),
        guard: Guard::BeforePullTransaction,
    };

    let error = check_purge_horizon_conformance(&adapter)
        .await
        .err()
        .ok_or("a guard outside the pull transaction must fail")?;
    assert_eq!(
        error.violations(),
        [
            "concurrent purge and pull: a pull that passed its cursor check omitted a tombstone \
          purged while it was open"
        ]
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn purge_tombstones_follows_the_caller_table_order() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    let expired = Utc::now() - TimeDelta::days(2);
    let cutoff = Utc::now() - TimeDelta::days(1);
    let (list, list_revision) = tombstone_list(&pool, owner, expired).await?;
    let mut item_revisions = Vec::new();
    for _ in 0..3 {
        item_revisions.push(tombstone_item(&pool, owner, list, expired).await?);
    }

    let parents_first = purge_tombstones(&pool, &[LISTS, ITEMS], cutoff, LIMIT).await?;
    assert_eq!(
        parents_first,
        [
            PurgedTable {
                table: "product_lists",
                deleted: 0
            },
            PurgedTable {
                table: "product_items",
                deleted: 3
            },
        ],
        "a parent with children is not selected, and items drain in batches of two"
    );
    assert_eq!(
        horizon(&pool, owner).await?,
        item_revisions.iter().copied().max()
    );

    let parents_after = purge_tombstones(&pool, &[ITEMS, LISTS], cutoff, LIMIT).await?;
    assert_eq!(
        parents_after,
        [
            PurgedTable {
                table: "product_items",
                deleted: 0
            },
            PurgedTable {
                table: "product_lists",
                deleted: 1
            },
        ]
    );
    let expected = item_revisions.iter().copied().chain([list_revision]).max();
    assert_eq!(horizon(&pool, owner).await?, expected);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_selector_that_ignores_the_limit_is_rejected_before_deleting() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    let expired = Utc::now() - TimeDelta::days(2);
    for _ in 0..3 {
        tombstone_record(&pool, owner, expired).await?;
    }
    let unbounded = TombstoneTable::new(
        "unbounded",
        "SELECT id, owner_id, revision FROM product_records
         WHERE deleted_at < $1 AND $2::bigint > 0
         FOR UPDATE SKIP LOCKED",
        "DELETE FROM product_records WHERE id = ANY($1)",
    );

    let mut transaction = pool.begin().await?;
    let result = purge_tombstone_batch(&mut transaction, &unbounded, Utc::now(), LIMIT).await;
    transaction.rollback().await?;

    assert!(matches!(
        result,
        Err(PurgeError::BatchLimitExceeded {
            table: "unbounded",
            selected: 3,
            ..
        })
    ));
    assert_eq!(record_count(&pool, owner).await?, 3);
    assert_eq!(horizon(&pool, owner).await?, None);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn an_owner_with_an_open_write_is_skipped_until_it_commits() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    let tombstone = tombstone_record(&pool, owner, Utc::now() - TimeDelta::days(2)).await?;

    let mut writer = pool.begin().await?;
    baukit_sync::next_revision(&mut writer, owner).await?;
    let mut purge = pool.begin().await?;
    let skipped = tokio::time::timeout(
        LOCK_WAIT,
        purge_tombstone_batch(&mut purge, &RECORDS, Utc::now(), LIMIT),
    )
    .await??;
    purge.commit().await?;
    assert_eq!((skipped.selected, skipped.deleted), (1, 0));
    assert!(!skipped.may_have_more(LIMIT));
    assert_eq!(horizon(&pool, owner).await?, None);
    writer.commit().await?;

    let mut purge = pool.begin().await?;
    let purged = purge_tombstone_batch(&mut purge, &RECORDS, Utc::now(), LIMIT).await?;
    purge.commit().await?;
    assert_eq!((purged.selected, purged.deleted), (1, 1));
    assert_eq!(horizon(&pool, owner).await?, Some(tombstone));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn an_open_pull_does_not_block_revision_allocation() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    let mut pull = pool.begin().await?;
    guard_pull_cursor(&mut pull, owner, 0).await?;
    let mut writer = pool.begin().await?;
    let revision =
        tokio::time::timeout(LOCK_WAIT, baukit_sync::next_revision(&mut writer, owner)).await??;
    writer.commit().await?;
    pull.commit().await?;

    assert_eq!(revision, 1);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_guard_rejects_negative_and_stale_cursors() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    tombstone_record(&pool, owner, Utc::now() - TimeDelta::days(2)).await?;
    let horizon_revision = tombstone_record(&pool, owner, Utc::now() - TimeDelta::days(2)).await?;
    purge_tombstones(&pool, &[RECORDS], Utc::now(), LIMIT).await?;

    let mut transaction = pool.begin().await?;
    let negative = guard_pull_cursor(&mut transaction, owner, -1).await;
    let stale = guard_pull_cursor(&mut transaction, owner, horizon_revision - 1).await;
    let at_horizon = guard_pull_cursor(&mut transaction, owner, horizon_revision).await;
    let full = guard_pull_cursor(&mut transaction, owner, 0).await;
    let unknown_owner = guard_pull_cursor(&mut transaction, Uuid::new_v4(), 7).await;
    transaction.commit().await?;

    assert!(matches!(
        negative,
        Err(PullGuardError::Cursor(PullCursorError::Negative))
    ));
    assert!(matches!(
        stale,
        Err(PullGuardError::Cursor(PullCursorError::ResyncRequired { horizon_revision: reported }))
            if reported == horizon_revision
    ));
    assert!(at_horizon.is_ok());
    assert!(full.is_ok());
    assert!(unknown_owner.is_ok());

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn the_horizon_table_rejects_a_non_positive_horizon() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    let zero =
        sqlx::query("INSERT INTO sync_purge_horizons (owner_id, horizon_revision) VALUES ($1, 0)")
            .bind(owner)
            .execute(&pool)
            .await;
    let orphan =
        sqlx::query("INSERT INTO sync_purge_horizons (owner_id, horizon_revision) VALUES ($1, 1)")
            .bind(Uuid::new_v4())
            .execute(&pool)
            .await;

    assert!(zero.is_err(), "a zero horizon must fail the check");
    assert!(
        orphan.is_err(),
        "a horizon needs the owner's revision counter"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn fixture() -> Result<(PostgresTestContainer, PgPool), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE product_records (
             id UUID PRIMARY KEY,
             owner_id UUID NOT NULL REFERENCES sync_revisions (owner_id) ON DELETE CASCADE,
             updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
             deleted_at TIMESTAMPTZ,
             revision BIGINT NOT NULL
         );
         CREATE INDEX product_records_sync_idx ON product_records (owner_id, revision);
         CREATE TABLE product_lists (
             id UUID PRIMARY KEY,
             owner_id UUID NOT NULL REFERENCES sync_revisions (owner_id) ON DELETE CASCADE,
             updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
             deleted_at TIMESTAMPTZ,
             revision BIGINT NOT NULL
         );
         CREATE TABLE product_items (
             id UUID PRIMARY KEY,
             list_id UUID NOT NULL REFERENCES product_lists (id),
             updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
             deleted_at TIMESTAMPTZ,
             revision BIGINT NOT NULL
         );",
    )
    .execute(&pool)
    .await?;
    Ok((fixture, pool))
}

async fn owner(pool: &PgPool) -> Result<Uuid, TestError> {
    let owner_id = Uuid::new_v4();
    let mut transaction = pool.begin().await?;
    baukit_sync::ensure_owner(&mut transaction, owner_id).await?;
    transaction.commit().await?;
    Ok(owner_id)
}

async fn tombstone_record(
    pool: &PgPool,
    owner: Uuid,
    deleted_at: DateTime<Utc>,
) -> Result<i64, TestError> {
    let mut transaction = pool.begin().await?;
    let id = Uuid::new_v4();
    let created = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query("INSERT INTO product_records (id, owner_id, revision) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(owner)
        .bind(created)
        .execute(&mut *transaction)
        .await?;
    let deleted = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query("UPDATE product_records SET deleted_at = $1, revision = $2 WHERE id = $3")
        .bind(deleted_at)
        .bind(deleted)
        .bind(id)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(deleted)
}

async fn tombstone_list(
    pool: &PgPool,
    owner: Uuid,
    deleted_at: DateTime<Utc>,
) -> Result<(Uuid, i64), TestError> {
    let mut transaction = pool.begin().await?;
    let id = Uuid::new_v4();
    let revision = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query(
        "INSERT INTO product_lists (id, owner_id, deleted_at, revision) VALUES ($1, $2, $3, $4)",
    )
    .bind(id)
    .bind(owner)
    .bind(deleted_at)
    .bind(revision)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok((id, revision))
}

async fn tombstone_item(
    pool: &PgPool,
    owner: Uuid,
    list: Uuid,
    deleted_at: DateTime<Utc>,
) -> Result<i64, TestError> {
    let mut transaction = pool.begin().await?;
    let revision = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query(
        "INSERT INTO product_items (id, list_id, deleted_at, revision) VALUES ($1, $2, $3, $4)",
    )
    .bind(Uuid::new_v4())
    .bind(list)
    .bind(deleted_at)
    .bind(revision)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    Ok(revision)
}

async fn horizon(pool: &PgPool, owner: Uuid) -> Result<Option<i64>, TestError> {
    let mut transaction = pool.begin().await?;
    let horizon = purge_horizon(&mut transaction, owner).await?;
    transaction.commit().await?;
    Ok(horizon)
}

async fn record_count(pool: &PgPool, owner: Uuid) -> Result<i64, TestError> {
    Ok(
        sqlx::query_scalar("SELECT count(*) FROM product_records WHERE owner_id = $1")
            .bind(owner)
            .fetch_one(pool)
            .await?,
    )
}
