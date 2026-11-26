#![cfg(test)]
#[path = "support/logs.rs"]
mod logs;
#[path = "support/postgres_fixture.rs"]
mod postgres_fixture;

use baukit_config::Secret;
use baukit_erasure::{
    ErasureFuture, ErasureService, ErasureState, IdentityDeletionError, IdentityRetention,
    PostgresErasureStore,
};
use baukit_jobs::{JobStore, PostgresJobStore};
use postgres_fixture::{Product, count, database, fixture};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    error::Error,
    sync::{Arc, Mutex},
    time::Duration,
};
use tracing::instrument::WithSubscriber as _;

struct BlockedDeleter;
impl baukit_erasure::IdentityAccountDeleter for BlockedDeleter {
    fn delete_account<'a>(
        &'a self,
        _subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(std::future::pending())
    }
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn inline_timeout_releases_job_lock_and_retains_durable_work() -> Result<(), Box<dyn Error>> {
    let (_container, pool, store, _fake, _service) = fixture().await?;
    let service = ErasureService::new(
        store.clone(),
        IdentityRetention::Delete {
            deleter: Arc::new(BlockedDeleter),
            provider_id: "test".into(),
            inline_timeout: Duration::from_millis(50),
            max_attempts: 2,
        },
    )?;
    let logs = logs::Logs::default();
    let outcome = service
        .erase("alice", None, "inline-timeout-001", &Product)
        .with_subscriber(logs.subscriber())
        .await?;
    assert_eq!(outcome.status, ErasureState::Pending);
    assert!(store.is_fenced("alice", None).await?);
    assert_eq!(count(&pool, "profiles").await?, 1);
    let mut transaction = pool.begin().await?;
    let status: String = sqlx::query_scalar("SELECT status FROM job_outbox FOR UPDATE NOWAIT")
        .fetch_one(&mut *transaction)
        .await?;
    assert_eq!(status, "pending");
    transaction.rollback().await?;
    let text = logs.text();
    assert!(text.contains(&outcome.operation_id.to_string()), "{text}");
    assert!(text.contains("error_class=\"timeout\""), "{text}");
    assert!(!text.contains("alice"), "{text}");
    Ok(())
}

const CONCURRENT_QUERY_BUDGET: Duration = Duration::from_secs(2);

/// Queries the single-connection pool and tries to claim the job from inside the provider call.
struct ProbingDeleter {
    pool: PgPool,
    observed: Mutex<Option<(bool, bool)>>,
}
impl baukit_erasure::IdentityAccountDeleter for ProbingDeleter {
    fn delete_account<'a>(
        &'a self,
        _subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(async move {
            let query = tokio::time::timeout(
                CONCURRENT_QUERY_BUDGET,
                sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(&self.pool),
            )
            .await;
            let connection_free = matches!(query, Ok(Ok(1)));
            let claimed = PostgresJobStore::new(self.pool.clone())
                .claim(
                    "competing-worker",
                    &[baukit_erasure::IDENTITY_DELETE_JOB_TYPE],
                    jiff::Timestamp::now(),
                    Duration::from_secs(5),
                )
                .await
                .map_err(|_| IdentityDeletionError::Retryable)?;
            *self.observed.lock().expect("observation lock") =
                Some((connection_free, claimed.is_some()));
            Ok(())
        })
    }
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn inline_provider_call_holds_no_connection_and_keeps_workers_off_the_job()
-> Result<(), Box<dyn Error>> {
    let (container, _pool, _store) = database().await?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(container.connection_url())
        .await?;
    let store = PostgresErasureStore::new(
        pool.clone(),
        Secret::new("test-key-with-at-least-32-bytes-of-entropy".into()),
    )?;
    let deleter = Arc::new(ProbingDeleter {
        pool: pool.clone(),
        observed: Mutex::new(None),
    });
    let service = ErasureService::new(
        store.clone(),
        IdentityRetention::Delete {
            deleter: deleter.clone(),
            provider_id: "test".into(),
            inline_timeout: Duration::from_secs(5),
            max_attempts: 2,
        },
    )?;
    let outcome = service
        .erase("alice", None, "inline-no-lock-001", &Product)
        .await?;
    let observed = *deleter.observed.lock().expect("observation lock");
    assert_eq!(
        observed,
        Some((true, false)),
        "a concurrent query succeeds on the one connection and no worker claims the job"
    );
    assert_eq!(outcome.status, ErasureState::Completed);
    assert_eq!(count(&pool, "job_outbox").await?, 0);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn a_failed_inline_attempt_makes_the_job_claimable_at_once() -> Result<(), Box<dyn Error>> {
    let (_container, pool, _store, fake, service) = fixture().await?;
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    let outcome = service
        .erase("alice", None, "inline-release-0001", &Product)
        .await?;
    assert_eq!(outcome.status, ErasureState::Pending);
    let claimed = PostgresJobStore::new(pool)
        .claim(
            "worker",
            &[baukit_erasure::IDENTITY_DELETE_JOB_TYPE],
            jiff::Timestamp::now(),
            Duration::from_secs(5),
        )
        .await?;
    assert!(
        claimed.is_some(),
        "the reservation ends with the failed attempt"
    );
    Ok(())
}
