#![cfg(test)]
use std::{error::Error, path::PathBuf, time::Duration};

use baukit_jobs::{CancellationOutcome, JobStore as _, NewJob, PostgresJobStore, StoreError};
use chrono::{TimeDelta, Utc};
use serde_json::json;
use sqlx::PgPool;

const CONCURRENCY_WAIT_LIMIT: Duration = Duration::from_secs(30);
const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(10);

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_cancellation_covers_pending_and_running_lifecycle() -> Result<(), Box<dyn Error>>
{
    let (fixture, pool, store) = fixture().await?;
    let pending = store
        .enqueue(NewJob::new("cancel.pending", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    assert!(store.request_cancellation(pending.id, now).await?);
    assert_eq!(status(&pool, pending.id).await?, "cancelled");

    let running = store
        .enqueue(NewJob::new("cancel.running", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim(
            "worker-a",
            &["cancel.running"],
            now,
            Duration::from_secs(30),
        )
        .await?
        .expect("running job claimed");
    assert!(store.request_cancellation(running.id, now).await?);
    assert!(store.cancellation_requested(running.id).await?);
    assert!(
        !store.complete(running.id, "worker-a", now).await?,
        "completion must not overtake cancellation"
    );
    assert!(store.cancel(running.id, "worker-a", now).await?);
    assert_eq!(status(&pool, running.id).await?, "cancelled");
    assert!(!store.request_cancellation(running.id, now).await?);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_key_cancellation_is_scoped_and_transactional() -> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let key = "connection:é%_:";
    for prefix in [false, true] {
        for lifecycle in ["pending", "running", "succeeded", "failed", "cancelled"] {
            let case = KeyCancellationCase {
                store: &store,
                pool: &pool,
                kind: format!("cancel.{prefix}.{lifecycle}"),
                key,
                prefix,
                lifecycle,
            };
            case.assert_cancellation().await?;
        }
    }
    pool.close().await;
    drop(fixture);
    Ok(())
}

struct KeyCancellationCase<'a> {
    store: &'a PostgresJobStore,
    pool: &'a PgPool,
    kind: String,
    key: &'a str,
    prefix: bool,
    lifecycle: &'a str,
}

impl KeyCancellationCase<'_> {
    async fn assert_cancellation(&self) -> Result<(), Box<dyn Error>> {
        let matching_key = if self.prefix {
            format!("{}slot:1", self.key)
        } else {
            self.key.to_owned()
        };
        let matching = self.keyed(&self.kind, &matching_key).await?;
        let untouched = self.untouched_jobs(&matching_key).await?;
        let before = job_state(self.pool, matching).await?;
        let now = Utc::now();
        let expected = CancellationOutcome {
            cancelled_pending: u64::from(self.lifecycle == "pending"),
            cancellation_requested_running: u64::from(self.lifecycle == "running"),
        };
        for commit in [false, true] {
            self.cancel_in_transaction(now, matching, &before, &expected, commit)
                .await?;
        }
        self.assert_matching_after(matching, &before, now).await?;
        for (id, state) in untouched {
            assert_eq!(
                job_state(self.pool, id).await?,
                state,
                "non-matching rows stay untouched"
            );
        }
        Ok(())
    }

    async fn keyed(&self, kind: &str, key: &str) -> Result<uuid::Uuid, Box<dyn Error>> {
        keyed_job(self.store, self.pool, kind, key, self.lifecycle).await
    }

    async fn snapshot(
        &self,
        id: uuid::Uuid,
    ) -> Result<(uuid::Uuid, serde_json::Value), Box<dyn Error>> {
        Ok((id, job_state(self.pool, id).await?))
    }

    async fn untouched_jobs(
        &self,
        matching_key: &str,
    ) -> Result<Vec<(uuid::Uuid, serde_json::Value)>, Box<dyn Error>> {
        let mut untouched = Vec::new();
        for other_key in ["connection:éanythingX:slot:2", "connection:é%_", "other"] {
            untouched.push(
                self.snapshot(self.keyed(&self.kind, other_key).await?)
                    .await?,
            );
        }
        if !self.prefix {
            let id = self
                .keyed(&self.kind, &format!("{}slot:2", self.key))
                .await?;
            untouched.push(self.snapshot(id).await?);
        }
        let other = self
            .keyed(&format!("{}.other", self.kind), matching_key)
            .await?;
        untouched.push(self.snapshot(other).await?);
        let unkeyed = self
            .store
            .enqueue(NewJob::new(&self.kind, json!({}), 3))
            .await?
            .job;
        untouched.push(self.snapshot(unkeyed.id).await?);
        Ok(untouched)
    }

    async fn cancel_in_transaction(
        &self,
        now: chrono::DateTime<Utc>,
        matching: uuid::Uuid,
        before: &serde_json::Value,
        expected: &CancellationOutcome,
        commit: bool,
    ) -> Result<(), Box<dyn Error>> {
        let mut transaction = self.pool.begin().await?;
        let outcome = if self.prefix {
            self.store
                .request_cancellation_by_key_prefix_in_transaction(
                    &mut transaction,
                    &self.kind,
                    self.key,
                    now,
                )
                .await?
        } else {
            self.store
                .request_cancellation_by_key_in_transaction(
                    &mut transaction,
                    &self.kind,
                    self.key,
                    now,
                )
                .await?
        };
        assert_eq!(&outcome, expected);
        assert_eq!(
            &job_state(self.pool, matching).await?,
            before,
            "changes are invisible before commit"
        );
        if commit {
            transaction.commit().await?;
        } else {
            transaction.rollback().await?;
            assert_eq!(
                &job_state(self.pool, matching).await?,
                before,
                "rollback preserves the entire row"
            );
        }
        Ok(())
    }

    async fn assert_matching_after(
        &self,
        matching: uuid::Uuid,
        before: &serde_json::Value,
        now: chrono::DateTime<Utc>,
    ) -> Result<(), Box<dyn Error>> {
        let after = job_state(self.pool, matching).await?;
        match self.lifecycle {
            "pending" => {
                assert_eq!(after["status"], "cancelled");
                assert!(after["locked_by"].is_null());
                assert!(after["locked_until"].is_null());
                assert!(after["cancel_requested_at"].is_null());
            }
            "running" => {
                assert_eq!(after["status"], "running");
                assert_eq!(after["locked_by"], before["locked_by"]);
                assert_eq!(after["locked_until"], before["locked_until"]);
                assert!(!after["cancel_requested_at"].is_null());
                assert!(self.store.cancellation_requested(matching).await?);
                assert!(!self.store.complete(matching, "cancel-worker", now).await?);
                assert!(self.store.cancel(matching, "cancel-worker", now).await?);
            }
            _ => assert_eq!(&after, before, "terminal rows stay untouched"),
        }
        Ok(())
    }
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_prefix_cancellation_counts_multiple_jobs_and_rejects_invalid_input()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    for (key, lifecycle) in [
        ("connection:%_:pending:1", "pending"),
        ("connection:%_:pending:2", "pending"),
        ("connection:%_:running:1", "running"),
        ("connection:%_:running:2", "running"),
        ("connection:%_:running:3", "running"),
    ] {
        keyed_job(&store, &pool, "cancel.batch", key, lifecycle).await?;
    }
    let now = Utc::now();
    let mut transaction = pool.begin().await?;
    assert_invalid_cancellation_input_is_rejected(&store, &mut transaction, now).await;
    assert_eq!(
        store
            .request_cancellation_by_key_prefix_in_transaction(
                &mut transaction,
                "cancel.batch",
                "missing",
                now
            )
            .await?,
        CancellationOutcome::default()
    );
    assert_eq!(
        store
            .request_cancellation_by_key_prefix_in_transaction(
                &mut transaction,
                "cancel.batch",
                "connection:%_:",
                now
            )
            .await?,
        CancellationOutcome {
            cancelled_pending: 2,
            cancellation_requested_running: 3
        }
    );
    assert_eq!(
        store
            .request_cancellation_by_key_prefix_in_transaction(
                &mut transaction,
                "cancel.batch",
                "connection:%_:",
                now
            )
            .await?,
        CancellationOutcome {
            cancelled_pending: 0,
            cancellation_requested_running: 3
        }
    );
    transaction.commit().await?;
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn assert_invalid_cancellation_input_is_rejected(
    store: &PostgresJobStore,
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    now: chrono::DateTime<Utc>,
) {
    for prefix in ["", " ", &"x".repeat(501)] {
        assert!(matches!(
            store
                .request_cancellation_by_key_prefix_in_transaction(
                    transaction,
                    "cancel.batch",
                    prefix,
                    now
                )
                .await,
            Err(StoreError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        store
            .request_cancellation_by_key_in_transaction(transaction, "", "key", now)
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store
            .request_cancellation_by_key_in_transaction(transaction, "cancel.batch", "", now)
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store
            .request_cancellation_by_key_prefix_in_transaction(transaction, "", "key", now)
            .await,
        Err(StoreError::InvalidInput(_))
    ));
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_transaction_lease_check_matches_completion_ownership()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store
        .enqueue(NewJob::new("lease.check", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    let mut transaction = pool.begin().await?;
    assert!(
        !store
            .check_lease_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    transaction.commit().await?;
    let claimed = store
        .claim("worker-a", &["lease.check"], now, Duration::from_secs(60))
        .await?
        .expect("job claimed");
    let deadline = claimed.locked_until.expect("lease deadline");
    let mut transaction = pool.begin().await?;
    assert!(
        store
            .check_lease_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    assert!(
        !store
            .check_lease_in_transaction(&mut transaction, job.id, "worker-b", now)
            .await?
    );
    for expired in [deadline, deadline + TimeDelta::seconds(1)] {
        assert!(
            !store
                .check_lease_in_transaction(&mut transaction, job.id, "worker-a", expired)
                .await?
        );
        assert!(
            !store
                .complete_in_transaction(&mut transaction, job.id, "worker-a", expired)
                .await?
        );
    }
    assert!(
        !store
            .check_lease_in_transaction(&mut transaction, uuid::Uuid::new_v4(), "worker-a", now)
            .await?
    );
    assert!(matches!(
        store
            .check_lease_in_transaction(&mut transaction, job.id, "", now)
            .await,
        Err(StoreError::InvalidInput(_))
    ));
    transaction.commit().await?;
    assert_eq!(
        status(&pool, job.id).await?,
        "running",
        "checking does not complete the job"
    );
    assert!(store.request_cancellation(job.id, now).await?);
    let mut transaction = pool.begin().await?;
    assert!(
        !store
            .check_lease_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    assert!(
        !store
            .complete_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    transaction.commit().await?;
    assert!(store.cancel(job.id, "worker-a", now).await?);
    assert_completed_job_has_no_lease(&store, &pool).await?;
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn assert_completed_job_has_no_lease(
    store: &PostgresJobStore,
    pool: &PgPool,
) -> Result<(), Box<dyn Error>> {
    let completed = store
        .enqueue(NewJob::new("lease.completed", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim(
            "worker-a",
            &["lease.completed"],
            now,
            Duration::from_secs(60),
        )
        .await?
        .expect("job claimed");
    assert!(store.complete(completed.id, "worker-a", now).await?);
    let mut transaction = pool.begin().await?;
    assert!(
        !store
            .check_lease_in_transaction(&mut transaction, completed.id, "worker-a", now)
            .await?
    );
    transaction.commit().await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn postgres_transaction_lease_check_blocks_cancellation_until_commit()
-> Result<(), Box<dyn Error>> {
    let (fixture, pool, store) = fixture().await?;
    let job = store
        .enqueue(NewJob::new("lease.lock", json!({}), 3))
        .await?
        .job;
    let now = Utc::now();
    store
        .claim("worker-a", &["lease.lock"], now, Duration::from_secs(60))
        .await?
        .expect("job claimed");
    let mut transaction = pool.begin().await?;
    assert!(
        store
            .check_lease_in_transaction(&mut transaction, job.id, "worker-a", now)
            .await?
    );
    let checker_pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *transaction)
        .await?;
    assert!(
        store
            .claim(
                "worker-b",
                &["lease.lock"],
                now + TimeDelta::seconds(60),
                Duration::from_secs(60)
            )
            .await?
            .is_none(),
        "claim skips the locked expired lease"
    );
    let cancellation_store = store.clone();
    let mut cancellation =
        tokio::spawn(async move { cancellation_store.request_cancellation(job.id, now).await });
    tokio::time::timeout(CONCURRENCY_WAIT_LIMIT, async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM pg_stat_activity WHERE $1 = ANY(pg_blocking_pids(pid)) AND wait_event_type = 'Lock')",
            ).bind(checker_pid).fetch_one(&pool).await?;
            if blocked {
                return Ok::<(), sqlx::Error>(());
            }
            assert!(!cancellation.is_finished(), "cancellation cannot finish while the check holds the lock");
            tokio::time::sleep(LOCK_POLL_INTERVAL).await;
        }
    }).await.expect("cancellation reaches the row lock before the wait limit")?;
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut cancellation)
            .await
            .is_err(),
        "a query observed waiting on the lock cannot finish before commit"
    );
    assert!(!store.cancellation_requested(job.id).await?);
    transaction.commit().await?;
    assert!(
        tokio::time::timeout(CONCURRENCY_WAIT_LIMIT, cancellation)
            .await
            .expect("cancellation finishes after commit")??
    );
    assert!(store.cancellation_requested(job.id).await?);
    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn fixture()
-> Result<(baukit_test::PostgresTestContainer, PgPool, PostgresJobStore), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    let store = PostgresJobStore::new(pool.clone());
    Ok((fixture, pool, store))
}

async fn status(pool: &PgPool, job_id: uuid::Uuid) -> Result<String, sqlx::Error> {
    sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}

async fn keyed_job(
    store: &PostgresJobStore,
    pool: &PgPool,
    kind: &str,
    key: &str,
    lifecycle: &str,
) -> Result<uuid::Uuid, Box<dyn Error>> {
    let job = store
        .enqueue(NewJob::new(kind, json!({}), 3).idempotency_key(key))
        .await?
        .job;
    let now = Utc::now();
    sqlx::query(
        "UPDATE job_outbox SET status = $2, locked_by = CASE WHEN $2 = 'running' THEN 'cancel-worker' END, locked_until = CASE WHEN $2 = 'running' THEN $3 END, failure_reason = CASE WHEN $2 = 'failed' THEN 'permanent' END, updated_at = $4 WHERE id = $1",
    ).bind(job.id).bind(lifecycle).bind(now + TimeDelta::minutes(5)).bind(now).execute(pool).await?;
    Ok(job.id)
}

async fn job_state(pool: &PgPool, job_id: uuid::Uuid) -> Result<serde_json::Value, sqlx::Error> {
    sqlx::query_scalar("SELECT to_jsonb(job_outbox) FROM job_outbox WHERE id = $1")
        .bind(job_id)
        .fetch_one(pool)
        .await
}
