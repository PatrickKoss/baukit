#![cfg(test)]
#[path = "support/postgres_database.rs"]
mod postgres_database;

use baukit_erasure::{
    ErasureError, ErasureFuture, ErasureService, IdentityRetention, PostgresErasureStore,
    ProductErasure, WriteFence, begin_fenced_write,
    internals::REJECTED_SESSION_SQL,
    kernel::{FenceRow, fence_rejects},
    reject_fenced_subject, with_write_fence,
};
use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use proptest::{
    prelude::*,
    strategy::ValueTree,
    test_runner::{Config, TestRunner},
};
use sqlx::{PgConnection, PgPool};
use std::{collections::HashMap, error::Error, time::Duration};
use uuid::Uuid;

const NOTES_TABLE: &str = "CREATE TABLE notes (subject TEXT NOT NULL)";
const LOCK_WAIT: Duration = Duration::from_millis(300);

struct NotesProduct;
impl ProductErasure for NotesProduct {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM notes WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}

async fn notes(pool: &PgPool, subject: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM notes WHERE subject = $1")
        .bind(subject)
        .fetch_one(pool)
        .await
}

async fn guarded_note(
    store: &PostgresErasureStore,
    subject: &str,
    authenticated_at: Option<Timestamp>,
) -> Result<(), ErasureError> {
    let mut transaction = store.begin_guarded_write(subject, authenticated_at).await?;
    sqlx::query("INSERT INTO notes (subject) VALUES ($1)")
        .bind(subject)
        .execute(&mut *transaction)
        .await?;
    transaction.commit().await?;
    Ok(())
}

fn earlier_login() -> Option<Timestamp> {
    Timestamp::now()
        .checked_sub(SignedDuration::from_mins(5))
        .ok()
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn write_admitted_before_an_erase_that_commits_first_is_rejected()
-> Result<(), Box<dyn Error>> {
    let (_container, pool, store) = postgres_database::database().await?;
    sqlx::raw_sql(NOTES_TABLE).execute(&pool).await?;
    let service = ErasureService::new(store.clone(), IdentityRetention::Retain)?;
    let login = earlier_login();
    reject_fenced_subject(&store, "alice", login).await?;
    service
        .erase("alice", login, "race-erase-first-01", &NotesProduct)
        .await?;
    assert!(matches!(
        guarded_note(&store, "alice", login).await,
        Err(ErasureError::ProfileErased)
    ));
    assert_eq!(notes(&pool, "alice").await?, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn a_scoped_write_is_fenced_and_an_unscoped_one_is_not() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store) = postgres_database::database().await?;
    sqlx::raw_sql(NOTES_TABLE).execute(&pool).await?;
    let service = ErasureService::new(store.clone(), IdentityRetention::Retain)?;
    let login = earlier_login();
    service
        .erase("alice", login, "scoped-write-fence-01", &NotesProduct)
        .await?;
    let scoped = with_write_fence(
        WriteFence::new(store.clone(), "alice", login),
        begin_fenced_write(&pool),
    )
    .await;
    assert!(matches!(scoped, Err(ErasureError::ProfileErased)));
    let bob = with_write_fence(
        WriteFence::new(store.clone(), "bob", login),
        begin_fenced_write(&pool),
    )
    .await?;
    bob.commit().await?;
    begin_fenced_write(&pool).await?.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn sign_in_with_the_erased_session_does_not_recreate_the_identity()
-> Result<(), Box<dyn Error>> {
    let (_container, pool, store) = postgres_database::database().await?;
    sqlx::raw_sql(NOTES_TABLE).execute(&pool).await?;
    sqlx::raw_sql("CREATE TABLE user_identities (subject TEXT PRIMARY KEY)")
        .execute(&pool)
        .await?;
    let service = ErasureService::new(store.clone(), IdentityRetention::Retain)?;
    let login = earlier_login();
    let resolve = |authenticated_at| {
        let (store, pool) = (store.clone(), pool.clone());
        async move {
            let mut transaction = pool.begin().await?;
            store
                .guard_subject(&mut transaction, "alice", authenticated_at)
                .await?;
            sqlx::query("INSERT INTO user_identities VALUES ('alice') ON CONFLICT DO NOTHING")
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            Ok::<_, ErasureError>(())
        }
    };
    resolve(login).await?;
    sqlx::query("DELETE FROM user_identities")
        .execute(&pool)
        .await?;
    service
        .erase("alice", login, "identity-recreate-01", &NotesProduct)
        .await?;
    for stale in [login, None] {
        assert!(matches!(
            resolve(stale).await,
            Err(ErasureError::ProfileErased)
        ));
    }
    let identities: i64 = sqlx::query_scalar("SELECT count(*) FROM user_identities")
        .fetch_one(&pool)
        .await?;
    assert_eq!(identities, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn erase_waits_for_a_guarded_write_and_then_deletes_its_rows() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store) = postgres_database::database().await?;
    sqlx::raw_sql(NOTES_TABLE).execute(&pool).await?;
    let service = ErasureService::new(store.clone(), IdentityRetention::Retain)?;
    let login = earlier_login();
    let mut write = store.begin_guarded_write("alice", login).await?;
    sqlx::query("INSERT INTO notes (subject) VALUES ('alice')")
        .execute(&mut *write)
        .await?;
    let erase = tokio::spawn({
        let service = service.clone();
        async move {
            service
                .erase("alice", login, "race-write-first-01", &NotesProduct)
                .await
        }
    });
    tokio::time::sleep(LOCK_WAIT).await;
    assert!(!erase.is_finished(), "erase must wait for the subject lock");
    write.commit().await?;
    erase.await??;
    assert_eq!(notes(&pool, "alice").await?, 0);
    Ok(())
}

#[derive(Clone, Debug)]
struct FenceCase {
    fence: Option<(i64, Vec<&'static str>)>,
    login: Option<i64>,
}

fn fence_case() -> impl Strategy<Value = FenceCase> {
    let offset = -3_i64..=3;
    let states = prop::collection::vec(
        prop::sample::select(vec!["pending", "completed", "failed"]),
        0..3,
    );
    (
        prop::option::of((offset.clone(), states)),
        prop::option::of(offset),
    )
        .prop_map(|(fence, login)| FenceCase { fence, login })
}

async fn insert_case(
    pool: &PgPool,
    hash: &[u8],
    base: Timestamp,
    case: &FenceCase,
) -> Result<(), Box<dyn Error>> {
    let Some((erased, states)) = &case.fence else {
        return Ok(());
    };
    sqlx::query("INSERT INTO erasure_fences (subject_hash, erased_at) VALUES ($1, $2)")
        .bind(hash)
        .bind((base + SignedDuration::from_secs(*erased)).to_sqlx())
        .execute(pool)
        .await?;
    for state in states {
        let key = [
            Uuid::new_v4().as_bytes().as_slice(),
            Uuid::new_v4().as_bytes(),
        ]
        .concat();
        sqlx::query("INSERT INTO erasure_operations (id, subject_hash, key_hash, state, response, completed_at) VALUES ($1, $2, $3, $4, '{}', CASE WHEN $4 = 'completed' THEN now() END)")
            .bind(Uuid::new_v4()).bind(hash).bind(key).bind(*state)
            .execute(pool).await?;
    }
    Ok(())
}

fn mirror(case: &FenceCase) -> bool {
    let fence = case.fence.as_ref().map(|(erased_at, states)| FenceRow {
        erased_at: *erased_at,
        unfinished_operation: states.iter().any(|state| *state != "completed"),
    });
    fence_rejects(fence, case.login)
}

/// K-22 agreement: the SQL fence query and its Rust mirror agree on generated rows.
#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn rejected_session_sql_agrees_with_the_mirror() -> Result<(), Box<dyn Error>> {
    let (_container, pool, _store) = postgres_database::database().await?;
    let base = Timestamp::now();
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::with_cases(1)
    });
    let cases: Vec<FenceCase> = (0..256)
        .map(|_| {
            fence_case()
                .new_tree(&mut runner)
                .map(|tree| tree.current())
        })
        .collect::<Result<_, _>>()
        .map_err(|reason| reason.to_string())?;
    for (index, case) in cases.iter().enumerate() {
        let hash = [Uuid::from_u128(index as u128).as_bytes().as_slice(); 2].concat();
        insert_case(&pool, &hash, base, case).await?;
        let login = case
            .login
            .map(|offset| (base + SignedDuration::from_secs(offset)).to_sqlx());
        let rejected: bool = sqlx::query_scalar(REJECTED_SESSION_SQL)
            .bind(vec![hash.clone()])
            .bind(login)
            .fetch_one(&pool)
            .await?;
        assert_eq!(rejected, mirror(case), "{case:?}");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug)]
enum Login {
    Missing,
    BeforeErasures,
    AfterErasures,
}

#[derive(Clone, Debug)]
enum Action {
    Erase {
        subject: usize,
        key: usize,
        login: Login,
    },
    Preflight {
        subject: usize,
        key: usize,
        login: Login,
    },
    Write {
        subject: usize,
        login: Login,
    },
    RacingWriteAndErase {
        subject: usize,
        key: usize,
    },
}

const SUBJECTS: [&str; 3] = ["alice", "bob", "carol"];
const KEYS: usize = 4;

fn action() -> impl Strategy<Value = Action> {
    let subject = 0..SUBJECTS.len();
    let key = 0..KEYS;
    let login = prop_oneof![
        Just(Login::Missing),
        Just(Login::BeforeErasures),
        Just(Login::AfterErasures)
    ];
    prop_oneof![
        (subject.clone(), key.clone(), login.clone()).prop_map(|(subject, key, login)| {
            Action::Erase {
                subject,
                key,
                login,
            }
        }),
        (subject.clone(), key.clone(), login.clone()).prop_map(|(subject, key, login)| {
            Action::Preflight {
                subject,
                key,
                login,
            }
        }),
        (subject.clone(), login).prop_map(|(subject, login)| Action::Write { subject, login }),
        (subject, key).prop_map(|(subject, key)| Action::RacingWriteAndErase { subject, key }),
    ]
}

#[derive(Default)]
struct Model {
    key_owner: HashMap<usize, usize>,
    receipts: HashMap<usize, Receipt>,
    fenced: [bool; SUBJECTS.len()],
    notes: [i64; SUBJECTS.len()],
}

type Receipt = (String, Uuid, Option<Timestamp>);

fn receipt(outcome: &baukit_erasure::ErasureOutcome) -> Receipt {
    (
        format!("{:?}", outcome.status),
        outcome.operation_id,
        outcome.completed_at,
    )
}

#[derive(Debug, PartialEq, Eq)]
enum Expected {
    Ok,
    Replay,
    Conflict,
    Fenced,
}

impl Model {
    fn rejects(&self, subject: usize, login: Login) -> bool {
        self.fenced[subject] && !matches!(login, Login::AfterErasures)
    }
    fn admission(&self, subject: usize, key: usize, login: Login) -> Expected {
        match self.key_owner.get(&key) {
            Some(owner) if *owner == subject => Expected::Replay,
            Some(_) => Expected::Conflict,
            None if self.rejects(subject, login) => Expected::Fenced,
            None => Expected::Ok,
        }
    }
}

struct Harness {
    pool: PgPool,
    store: PostgresErasureStore,
    service: ErasureService,
    epoch: Timestamp,
}

impl Harness {
    fn login(&self, login: Login) -> Option<Timestamp> {
        let hour = SignedDuration::from_hours(1);
        match login {
            Login::Missing => None,
            Login::BeforeErasures => Some(self.epoch - hour),
            Login::AfterErasures => Some(Timestamp::now() + hour),
        }
    }

    fn key(key: usize) -> String {
        format!("model-erasure-key-{key:04}")
    }

    async fn reset(&self) -> Result<(), sqlx::Error> {
        sqlx::raw_sql("TRUNCATE notes, erasure_operations, erasure_fences, job_outbox")
            .execute(&self.pool)
            .await
            .map(|_| ())
    }

    async fn erase(
        &self,
        model: &mut Model,
        subject: usize,
        key: usize,
        login: Login,
    ) -> Result<(), Box<dyn Error>> {
        let expected = model.admission(subject, key, login);
        let operations = self.operation_count().await?;
        let result = self
            .service
            .erase(
                SUBJECTS[subject],
                self.login(login),
                &Self::key(key),
                &NotesProduct,
            )
            .await;
        self.check_erase(model, subject, key, expected, result, operations)
            .await?;
        Ok(())
    }

    async fn operation_count(&self) -> Result<i64, sqlx::Error> {
        sqlx::query_scalar("SELECT count(*) FROM erasure_operations")
            .fetch_one(&self.pool)
            .await
    }

    /// `erase` answers a same-key replay with the first receipt, so a replay must return
    /// `Ok` carrying that exact receipt, add no operation and rerun no product erase.
    async fn check_erase(
        &self,
        model: &mut Model,
        subject: usize,
        key: usize,
        expected: Expected,
        result: Result<baukit_erasure::ErasureOutcome, ErasureError>,
        operations_before: i64,
    ) -> Result<(), Box<dyn Error>> {
        let outcome = match result {
            Ok(outcome) => outcome,
            Err(error) => {
                assert_eq!(classify(Err(error))?, expected);
                assert_eq!(self.operation_count().await?, operations_before);
                return Ok(());
            }
        };
        match expected {
            Expected::Replay => {
                assert_eq!(Some(&receipt(&outcome)), model.receipts.get(&key));
                assert_eq!(self.operation_count().await?, operations_before);
            }
            Expected::Ok => {
                assert_eq!(self.operation_count().await?, operations_before + 1);
                model.receipts.insert(key, receipt(&outcome));
                model.key_owner.insert(key, subject);
                model.fenced[subject] = true;
                model.notes[subject] = 0;
            }
            other => panic!("erase succeeded where the model expected {other:?}"),
        }
        Ok(())
    }

    async fn apply(&self, model: &mut Model, action: Action) -> Result<(), Box<dyn Error>> {
        match action {
            Action::Erase {
                subject,
                key,
                login,
            } => self.erase(model, subject, key, login).await?,
            Action::Preflight {
                subject,
                key,
                login,
            } => {
                let expected = model.admission(subject, key, login);
                let result = self
                    .service
                    .preflight(SUBJECTS[subject], self.login(login), &Self::key(key))
                    .await;
                let actual = match result {
                    Ok(baukit_erasure::ErasurePreflight::Replay(_)) => Expected::Replay,
                    other => classify(other.map(|_| ()))?,
                };
                assert_eq!(actual, expected);
            }
            Action::Write { subject, login } => {
                let result = guarded_note(&self.store, SUBJECTS[subject], self.login(login)).await;
                let expected = if model.rejects(subject, login) {
                    Expected::Fenced
                } else {
                    model.notes[subject] += 1;
                    Expected::Ok
                };
                assert_eq!(classify(result)?, expected);
            }
            Action::RacingWriteAndErase { subject, key } => {
                self.race(model, subject, key).await?;
            }
        }
        for (index, subject) in SUBJECTS.iter().enumerate() {
            assert_eq!(notes(&self.pool, subject).await?, model.notes[index]);
        }
        Ok(())
    }

    async fn race(
        &self,
        model: &mut Model,
        subject: usize,
        key: usize,
    ) -> Result<(), Box<dyn Error>> {
        let expected = model.admission(subject, key, Login::BeforeErasures);
        let login = self.login(Login::BeforeErasures);
        let key_text = Self::key(key);
        let write = guarded_note(&self.store, SUBJECTS[subject], login);
        let erase = self
            .service
            .erase(SUBJECTS[subject], login, &key_text, &NotesProduct);
        let operations = self.operation_count().await?;
        let (write, erase) = tokio::join!(write, erase);
        let wrote = classify(write)? == Expected::Ok;
        let erased = expected == Expected::Ok;
        if !erased {
            assert_eq!(wrote, !model.rejects(subject, Login::BeforeErasures));
            model.notes[subject] += i64::from(wrote);
        }
        self.check_erase(model, subject, key, expected, erase, operations)
            .await?;
        Ok(())
    }
}

fn classify(result: Result<(), ErasureError>) -> Result<Expected, ErasureError> {
    match result {
        Ok(()) => Ok(Expected::Ok),
        Err(ErasureError::Conflict) => Ok(Expected::Conflict),
        Err(ErasureError::ProfileErased) => Ok(Expected::Fenced),
        Err(error) => Err(error),
    }
}

/// P-14: erase, preflight and guarded writes against a replay, conflict and fence model.
#[test]
#[ignore = "requires Docker PostgreSQL"]
fn erasure_service_matches_the_model() -> Result<(), Box<dyn Error>> {
    let runtime = tokio::runtime::Runtime::new()?;
    let (container, pool, store) = runtime.block_on(postgres_database::database())?;
    runtime.block_on(sqlx::raw_sql(NOTES_TABLE).execute(&pool))?;
    let harness = Harness {
        service: ErasureService::new(store.clone(), IdentityRetention::Retain)?,
        pool,
        store,
        epoch: Timestamp::now(),
    };
    let mut runner = TestRunner::new(Config {
        failure_persistence: None,
        ..Config::with_cases(24)
    });
    runner.run(&prop::collection::vec(action(), 1..16), |actions| {
        runtime
            .block_on(async {
                harness.reset().await?;
                let mut model = Model::default();
                for action in actions {
                    harness.apply(&mut model, action).await?;
                }
                Ok::<_, Box<dyn Error>>(())
            })
            .map_err(|error| TestCaseError::fail(error.to_string()))
    })?;
    runtime.block_on(async move { drop(container) });
    Ok(())
}
