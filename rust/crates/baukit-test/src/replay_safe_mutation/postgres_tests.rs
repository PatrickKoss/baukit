use ring::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};
use serde_json::json;
use sqlx::{PgPool, Postgres, Row as _, Transaction};

use super::*;

const CREATED: u16 = 201;
const AS_APP: &str = "SET LOCAL ROLE conformance_app";
const AS_JANITOR: &str = "SET LOCAL ROLE conformance_janitor";
const COUNT_EFFECTS: &str = "SELECT count(*) FROM conformance_effects WHERE owner_id = $1";
const COUNT_REPLAYS: &str = "SELECT count(*) FROM conformance_replays WHERE owner_id = $1";

const SCHEMA: [&str; 12] = [
    "CREATE ROLE conformance_app NOLOGIN",
    "CREATE ROLE conformance_janitor NOLOGIN",
    "CREATE TABLE conformance_owners (id uuid PRIMARY KEY)",
    "CREATE TABLE conformance_effects (\
         id uuid PRIMARY KEY DEFAULT gen_random_uuid(), \
         owner_id uuid NOT NULL REFERENCES conformance_owners(id) ON DELETE CASCADE, \
         operation text NOT NULL, title text NOT NULL, amount bigint NOT NULL\
     )",
    "CREATE TABLE conformance_replays (\
         owner_id uuid NOT NULL REFERENCES conformance_owners(id) ON DELETE CASCADE, \
         operation text NOT NULL, \
         key_digest bytea NOT NULL CHECK (octet_length(key_digest) = 32), \
         fingerprint bytea NOT NULL CHECK (octet_length(fingerprint) = 32), \
         response_status smallint, \
         response_body jsonb, \
         expires_at timestamptz NOT NULL, \
         PRIMARY KEY (owner_id, operation, key_digest)\
     )",
    "CREATE INDEX conformance_replays_expiry ON conformance_replays (expires_at)",
    "ALTER TABLE conformance_replays ENABLE ROW LEVEL SECURITY",
    "ALTER TABLE conformance_replays FORCE ROW LEVEL SECURITY",
    "CREATE POLICY conformance_replays_owner ON conformance_replays TO conformance_app \
     USING (owner_id = nullif(current_setting('app.owner_id', true), '')::uuid) \
     WITH CHECK (owner_id = nullif(current_setting('app.owner_id', true), '')::uuid)",
    "CREATE POLICY conformance_replays_cleanup ON conformance_replays \
     FOR ALL TO conformance_janitor USING (expires_at < now())",
    "GRANT SELECT, INSERT, DELETE ON conformance_owners, conformance_effects TO conformance_app",
    "GRANT SELECT, INSERT, UPDATE, DELETE ON conformance_replays \
     TO conformance_app, conformance_janitor",
];

#[derive(Debug, thiserror::Error)]
enum SqlReplayError {
    #[error("database operation failed")]
    Database(#[from] sqlx::Error),
    #[error("request body is not valid")]
    Body(#[from] serde_json::Error),
    #[error("injected failure before commit")]
    Injected,
    #[error("the claimed replay record disappeared")]
    MissingRecord,
    #[error("stored status is out of range")]
    Status,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct NoteInput {
    title: String,
    amount: i64,
}

struct SqlReplayStore {
    pool: PgPool,
    cleanup_role: &'static str,
}

enum Claim {
    New,
    Stored(ReplayOutcome),
}

impl SqlReplayStore {
    async fn as_owner(&self, owner: Uuid) -> Result<Transaction<'static, Postgres>, sqlx::Error> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(AS_APP).execute(&mut *transaction).await?;
        sqlx::query("SELECT set_config('app.owner_id', $1, true)")
            .bind(owner.to_string())
            .execute(&mut *transaction)
            .await?;
        Ok(transaction)
    }

    async fn claim(
        transaction: &mut Transaction<'static, Postgres>,
        scope: &ReplayScope,
    ) -> Result<Claim, SqlReplayError> {
        let inserted = sqlx::query(
            "INSERT INTO conformance_replays \
                 (owner_id, operation, key_digest, fingerprint, expires_at) \
             VALUES ($1, $2, $3, $4, now() + interval '7 days') \
             ON CONFLICT DO NOTHING RETURNING 1",
        )
        .bind(scope.owner)
        .bind(scope.operation)
        .bind(&scope.key_digest)
        .bind(&scope.fingerprint)
        .fetch_optional(&mut **transaction)
        .await?;
        if inserted.is_some() {
            return Ok(Claim::New);
        }
        let row = sqlx::query(
            "SELECT fingerprint, response_status, response_body, expires_at <= now() AS expired \
             FROM conformance_replays \
             WHERE owner_id = $1 AND operation = $2 AND key_digest = $3 FOR UPDATE",
        )
        .bind(scope.owner)
        .bind(scope.operation)
        .bind(&scope.key_digest)
        .fetch_optional(&mut **transaction)
        .await?
        .ok_or(SqlReplayError::MissingRecord)?;
        if row.try_get::<bool, _>("expired")? {
            Self::reset_expired(transaction, scope).await?;
            return Ok(Claim::New);
        }
        if row.try_get::<Vec<u8>, _>("fingerprint")? != scope.fingerprint {
            return Ok(Claim::Stored(ReplayOutcome::Conflict));
        }
        let status = row.try_get::<i16, _>("response_status")?;
        let snapshot = ReplaySnapshot {
            status: u16::try_from(status).map_err(|_| SqlReplayError::Status)?,
            body: row.try_get("response_body")?,
        };
        Ok(Claim::Stored(ReplayOutcome::Replayed(snapshot)))
    }

    async fn reset_expired(
        transaction: &mut Transaction<'static, Postgres>,
        scope: &ReplayScope,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE conformance_replays \
             SET fingerprint = $4, response_status = NULL, response_body = NULL, \
                 expires_at = now() + interval '7 days' \
             WHERE owner_id = $1 AND operation = $2 AND key_digest = $3",
        )
        .bind(scope.owner)
        .bind(scope.operation)
        .bind(&scope.key_digest)
        .bind(&scope.fingerprint)
        .execute(&mut **transaction)
        .await?;
        Ok(())
    }

    async fn apply(
        transaction: &mut Transaction<'static, Postgres>,
        scope: &ReplayScope,
        input: &NoteInput,
    ) -> Result<ReplaySnapshot, SqlReplayError> {
        let id = sqlx::query_scalar::<_, Uuid>(
            "INSERT INTO conformance_effects (owner_id, operation, title, amount) \
             VALUES ($1, $2, $3, $4) RETURNING id",
        )
        .bind(scope.owner)
        .bind(scope.operation)
        .bind(&input.title)
        .bind(input.amount)
        .fetch_one(&mut **transaction)
        .await?;
        let snapshot = ReplaySnapshot {
            status: CREATED,
            body: json!({ "id": id, "title": input.title, "amount": input.amount }),
        };
        sqlx::query(
            "UPDATE conformance_replays SET response_status = $4, response_body = $5 \
             WHERE owner_id = $1 AND operation = $2 AND key_digest = $3",
        )
        .bind(scope.owner)
        .bind(scope.operation)
        .bind(&scope.key_digest)
        .bind(i16::try_from(snapshot.status).map_err(|_| SqlReplayError::Status)?)
        .bind(&snapshot.body)
        .execute(&mut **transaction)
        .await?;
        Ok(snapshot)
    }

    async fn count(&self, statement: &'static str, owner: Uuid) -> Result<u64, SqlReplayError> {
        let count = sqlx::query_scalar::<_, i64>(statement)
            .bind(owner)
            .fetch_one(&self.pool)
            .await?;
        Ok(u64::try_from(count).unwrap_or(u64::MAX))
    }
}

struct ReplayScope {
    owner: Uuid,
    operation: &'static str,
    key_digest: Vec<u8>,
    fingerprint: Vec<u8>,
}

impl ReplayScope {
    fn new(request: &ReplayRequest<'_, Uuid>, input: &NoteInput) -> Result<Self, SqlReplayError> {
        let operation = match request.operation {
            ReplayOperation::Primary => "create_note",
            ReplayOperation::Secondary => "create_reminder",
        };
        let canonical = serde_json::to_vec(&json!({
            "operation": operation,
            "input": serde_json::to_value(input)?,
        }))?;
        Ok(Self {
            owner: *request.owner,
            operation,
            key_digest: digest(&SHA256, request.key.as_bytes()).as_ref().to_vec(),
            fingerprint: digest(&SHA256, &canonical).as_ref().to_vec(),
        })
    }
}

impl ReplaySafeMutationAdapter for SqlReplayStore {
    type Owner = Uuid;
    type Error = SqlReplayError;

    async fn create_owner(&self) -> Result<Uuid, SqlReplayError> {
        let owner = Uuid::new_v4();
        sqlx::query("INSERT INTO conformance_owners (id) VALUES ($1)")
            .bind(owner)
            .execute(&self.pool)
            .await?;
        Ok(owner)
    }

    async fn execute(
        &self,
        request: ReplayRequest<'_, Uuid>,
        checkpoint: CommitCheckpoint,
    ) -> Result<ReplayOutcome, SqlReplayError> {
        let input = serde_json::from_str::<NoteInput>(request.body)?;
        let scope = ReplayScope::new(&request, &input)?;
        let mut transaction = self.as_owner(scope.owner).await?;
        if let Claim::Stored(outcome) = Self::claim(&mut transaction, &scope).await? {
            transaction.rollback().await?;
            return Ok(outcome);
        }
        let snapshot = Self::apply(&mut transaction, &scope, &input).await?;
        if checkpoint.reached().await.is_err() {
            transaction.rollback().await?;
            return Err(SqlReplayError::Injected);
        }
        transaction.commit().await?;
        Ok(ReplayOutcome::Applied(snapshot))
    }

    async fn effects(&self, owner: &Uuid) -> Result<u64, SqlReplayError> {
        self.count(COUNT_EFFECTS, *owner).await
    }

    async fn replay_records(&self, owner: &Uuid) -> Result<u64, SqlReplayError> {
        self.count(COUNT_REPLAYS, *owner).await
    }

    async fn expire_replay_records(&self, owner: &Uuid) -> Result<(), SqlReplayError> {
        sqlx::query(
            "UPDATE conformance_replays SET expires_at = now() - interval '1 second' \
             WHERE owner_id = $1",
        )
        .bind(owner)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn purge_expired(&self, limit: NonZeroU32) -> Result<u64, SqlReplayError> {
        let mut transaction = self.pool.begin().await?;
        sqlx::query(self.cleanup_role)
            .execute(&mut *transaction)
            .await?;
        let deleted = sqlx::query(
            "DELETE FROM conformance_replays \
             WHERE (owner_id, operation, key_digest) IN (\
                 SELECT owner_id, operation, key_digest FROM conformance_replays \
                 WHERE expires_at < now() ORDER BY expires_at \
                 LIMIT $1 FOR UPDATE SKIP LOCKED\
             )",
        )
        .bind(i64::from(limit.get()))
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        transaction.commit().await?;
        Ok(deleted)
    }

    async fn erase_owner(&self, owner: &Uuid) -> Result<(), SqlReplayError> {
        let mut transaction = self.as_owner(*owner).await?;
        sqlx::query("DELETE FROM conformance_owners WHERE id = $1")
            .bind(owner)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn discard_transient_state(&self) -> Result<(), SqlReplayError> {
        Ok(())
    }
}

fn inputs() -> ReplayConformanceInputs {
    ReplayConformanceInputs {
        input: r#"{"title":"standup","amount":3}"#.to_owned(),
        equivalent_input: "{ \"amount\": 3,\n  \"title\": \"standup\" }".to_owned(),
        changed_input: r#"{"title":"standup","amount":4}"#.to_owned(),
        secondary_input: r#"{"title":"standup","amount":3}"#.to_owned(),
    }
}

async fn prepared_pool() -> Result<(crate::PostgresTestContainer, PgPool), Box<dyn StdError>> {
    let fixture = crate::start_postgres().await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    for statement in SCHEMA {
        sqlx::query(statement).execute(&pool).await?;
    }
    Ok((fixture, pool))
}

#[tokio::test]
#[ignore = "requires a reachable Docker daemon for PostgreSQL replay-safe mutation coverage"]
async fn postgres_reference_adapter_conforms() -> Result<(), Box<dyn StdError>> {
    let (fixture, pool) = prepared_pool().await?;
    let store = SqlReplayStore {
        pool: pool.clone(),
        cleanup_role: AS_JANITOR,
    };

    check_replay_safe_mutation_conformance(&store, &inputs()).await?;

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires a reachable Docker daemon for PostgreSQL replay-safe mutation coverage"]
async fn cleanup_without_a_cleanup_policy_is_reported() -> Result<(), Box<dyn StdError>> {
    let (fixture, pool) = prepared_pool().await?;
    let store = SqlReplayStore {
        pool: pool.clone(),
        cleanup_role: AS_APP,
    };

    let error = check_replay_safe_mutation_conformance(&store, &inputs())
        .await
        .expect_err("owner-scoped cleanup sees no rows under row-level security");

    assert_eq!(
        error.violations(),
        [
            "bounded cleanup: a batch with limit 2 deleted 0 records; expected 2",
            "bounded cleanup: cleanup left expired records",
        ]
    );
    pool.close().await;
    drop(fixture);
    Ok(())
}
