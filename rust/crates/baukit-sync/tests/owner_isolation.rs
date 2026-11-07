use std::{error::Error, num::NonZeroU32, path::PathBuf};

use baukit_sync::purge::{
    PullGuardError, TombstoneTable, guard_pull_cursor, purge_horizon, purge_tombstone_batch,
};
use chrono::{TimeDelta, Utc};
use proptest::{
    prelude::*,
    test_runner::{Config, TestCaseError, TestRunner},
};
use sqlx::PgPool;
use tokio::runtime::Runtime;
use uuid::Uuid;

type TestError = Box<dyn Error + Send + Sync>;

const CASES: u32 = 32;
const MAX_OPERATIONS: usize = 24;
const SHARED_IDS: u8 = 4;
const LIMIT: NonZeroU32 = NonZeroU32::new(3).expect("three is not zero");

const LANE_A: TombstoneTable = TombstoneTable::new(
    "lane_a",
    "SELECT id, owner_id, revision FROM isolated_records
     WHERE lane = 'a' AND deleted_at < $1
     ORDER BY deleted_at, id LIMIT $2 FOR UPDATE SKIP LOCKED",
    DELETE_PURGED,
);

const LANE_B: TombstoneTable = TombstoneTable::new(
    "lane_b",
    "SELECT id, owner_id, revision FROM isolated_records
     WHERE lane = 'b' AND deleted_at < $1
     ORDER BY deleted_at, id LIMIT $2 FOR UPDATE SKIP LOCKED",
    DELETE_PURGED,
);

const DELETE_PURGED: &str = "DELETE FROM isolated_records record
     USING UNNEST($1::uuid[], $2::uuid[]) AS purged (id, owner_id)
     WHERE record.id = purged.id AND record.owner_id = purged.owner_id";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Actor {
    A,
    B,
}

#[derive(Clone, Copy, Debug)]
enum Operation {
    EnsureOwner,
    Write { id: u8, tombstone: bool },
    CurrentRevision,
    Purge,
    Pull { cursor: i64 },
}

fn operation() -> impl Strategy<Value = Operation> {
    prop_oneof![
        Just(Operation::EnsureOwner),
        (0..SHARED_IDS, any::<bool>())
            .prop_map(|(id, tombstone)| Operation::Write { id, tombstone }),
        Just(Operation::CurrentRevision),
        Just(Operation::Purge),
        (0_i64..8).prop_map(|cursor| Operation::Pull { cursor }),
    ]
}

fn step() -> impl Strategy<Value = (Actor, Operation)> {
    (prop_oneof![Just(Actor::A), Just(Actor::B)], operation())
}

#[derive(Debug, Eq, PartialEq)]
struct OwnerState {
    counter: Option<i64>,
    horizon: Option<i64>,
    rows: Vec<(Uuid, i64, bool)>,
}

struct Case {
    pool: PgPool,
    owners: [Uuid; 2],
    ids: Vec<Uuid>,
}

impl Case {
    fn owner(&self, actor: Actor) -> Uuid {
        match actor {
            Actor::A => self.owners[0],
            Actor::B => self.owners[1],
        }
    }

    fn other(actor: Actor) -> Actor {
        match actor {
            Actor::A => Actor::B,
            Actor::B => Actor::A,
        }
    }

    async fn state(&self, actor: Actor) -> Result<OwnerState, TestError> {
        let owner = self.owner(actor);
        let mut transaction = self.pool.begin().await?;
        let counter = baukit_sync::current_revision(&mut transaction, owner).await?;
        let horizon = purge_horizon(&mut transaction, owner).await?;
        let rows = sqlx::query_as(
            "SELECT id, revision, deleted_at IS NOT NULL FROM isolated_records
             WHERE owner_id = $1 ORDER BY id",
        )
        .bind(owner)
        .fetch_all(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(OwnerState {
            counter,
            horizon,
            rows,
        })
    }

    async fn run(&self, actor: Actor, operation: Operation) -> Result<(), TestError> {
        let owner = self.owner(actor);
        let mut transaction = self.pool.begin().await?;
        match operation {
            Operation::EnsureOwner => baukit_sync::ensure_owner(&mut transaction, owner).await?,
            Operation::CurrentRevision => {
                baukit_sync::current_revision(&mut transaction, owner).await?;
            }
            Operation::Write { id, tombstone } => {
                let id = self.ids.get(usize::from(id)).copied().ok_or("id index")?;
                write(&mut transaction, actor, owner, id, tombstone).await?;
            }
            Operation::Purge => {
                let table = match actor {
                    Actor::A => &LANE_A,
                    Actor::B => &LANE_B,
                };
                purge_tombstone_batch(&mut transaction, table, Utc::now(), LIMIT).await?;
            }
            Operation::Pull { cursor } => pull(&mut transaction, owner, cursor).await?,
        }
        transaction.commit().await?;
        Ok(())
    }
}

async fn write(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Actor,
    owner: Uuid,
    id: Uuid,
    tombstone: bool,
) -> Result<(), TestError> {
    let lane = match actor {
        Actor::A => "a",
        Actor::B => "b",
    };
    let revision = baukit_sync::next_revision(transaction, owner).await?;
    let deleted_at = tombstone.then(|| Utc::now() - TimeDelta::hours(1));
    sqlx::query(
        "INSERT INTO isolated_records (owner_id, id, lane, deleted_at, revision)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (owner_id, id) DO UPDATE
         SET deleted_at = EXCLUDED.deleted_at, revision = EXCLUDED.revision, updated_at = now()
         WHERE isolated_records.owner_id = EXCLUDED.owner_id",
    )
    .bind(owner)
    .bind(id)
    .bind(lane)
    .bind(deleted_at)
    .bind(revision)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn pull(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    owner: Uuid,
    cursor: i64,
) -> Result<(), TestError> {
    match guard_pull_cursor(transaction, owner, cursor).await {
        Ok(()) => {}
        Err(PullGuardError::Cursor(_)) => return Ok(()),
        Err(error) => return Err(error.into()),
    }
    let owners: Vec<Uuid> = sqlx::query_scalar(
        "SELECT owner_id FROM isolated_records WHERE owner_id = $1 AND revision > $2",
    )
    .bind(owner)
    .bind(cursor)
    .fetch_all(&mut **transaction)
    .await?;
    if owners.iter().any(|row_owner| *row_owner != owner) {
        return Err("pull returned another owner's row".into());
    }
    Ok(())
}

async fn check_case(pool: &PgPool, steps: &[(Actor, Operation)]) -> Result<(), TestError> {
    let owners = [Uuid::new_v4(), Uuid::new_v4()];
    let ids = (0..SHARED_IDS).map(|_| Uuid::new_v4()).collect();
    let case = Case {
        pool: pool.clone(),
        owners,
        ids,
    };
    for owner in owners {
        let mut transaction = pool.begin().await?;
        baukit_sync::ensure_owner(&mut transaction, owner).await?;
        transaction.commit().await?;
    }
    for id in 0..SHARED_IDS {
        case.run(
            Actor::A,
            Operation::Write {
                id,
                tombstone: true,
            },
        )
        .await?;
        case.run(
            Actor::B,
            Operation::Write {
                id,
                tombstone: true,
            },
        )
        .await?;
    }
    for &(actor, operation) in steps {
        let bystander = Case::other(actor);
        let before = case.state(bystander).await?;
        case.run(actor, operation).await?;
        let after = case.state(bystander).await?;
        if before != after {
            return Err(format!(
                "{actor:?} {operation:?} changed {bystander:?}: {before:?} -> {after:?}"
            )
            .into());
        }
    }
    Ok(())
}

async fn fixture() -> Result<(baukit_test::PostgresTestContainer, PgPool), TestError> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::raw_sql(
        "CREATE TABLE isolated_records (
             owner_id UUID NOT NULL REFERENCES sync_revisions (owner_id),
             id UUID NOT NULL,
             lane TEXT NOT NULL,
             updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
             deleted_at TIMESTAMPTZ,
             revision BIGINT NOT NULL,
             PRIMARY KEY (owner_id, id)
         );
         CREATE INDEX isolated_records_sync_idx ON isolated_records (owner_id, revision);",
    )
    .execute(&pool)
    .await?;
    Ok((fixture, pool))
}

#[test]
#[ignore = "requires Docker PostgreSQL"]
fn one_owners_operations_never_change_the_other_owner() -> Result<(), TestError> {
    let runtime = Runtime::new()?;
    let _context = runtime.enter();
    let (fixture, pool) = runtime.block_on(fixture())?;
    let mut runner = TestRunner::new(Config {
        cases: CASES,
        failure_persistence: None,
        ..Config::default()
    });
    let result = runner.run(
        &proptest::collection::vec(step(), 1..MAX_OPERATIONS),
        |steps| {
            runtime
                .block_on(check_case(&pool, &steps))
                .map_err(|error| TestCaseError::fail(error.to_string()))
        },
    );
    runtime.block_on(pool.close());
    drop(fixture);
    result.map_err(|error| error.to_string().into())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn purge_keeps_another_owners_row_with_the_same_id() -> Result<(), TestError> {
    let (fixture, pool) = fixture().await?;
    let case = Case {
        pool: pool.clone(),
        owners: [Uuid::new_v4(), Uuid::new_v4()],
        ids: vec![Uuid::new_v4()],
    };
    case.run(Actor::A, Operation::EnsureOwner).await?;
    case.run(Actor::B, Operation::EnsureOwner).await?;
    case.run(
        Actor::A,
        Operation::Write {
            id: 0,
            tombstone: true,
        },
    )
    .await?;
    case.run(
        Actor::B,
        Operation::Write {
            id: 0,
            tombstone: true,
        },
    )
    .await?;
    let before = case.state(Actor::B).await?;

    case.run(Actor::A, Operation::Purge).await?;

    assert_eq!(case.state(Actor::A).await?.rows, Vec::new());
    assert_eq!(case.state(Actor::B).await?, before);
    pool.close().await;
    drop(fixture);
    Ok(())
}
