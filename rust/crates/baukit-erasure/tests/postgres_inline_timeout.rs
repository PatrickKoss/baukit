#[path = "support/logs.rs"]
mod logs;
#[path = "support/postgres_fixture.rs"]
mod postgres_fixture;

use baukit_erasure::{ErasureFuture, ErasureService, ErasureState, IdentityDeletionError};
use postgres_fixture::{Product, count, fixture};
use std::{error::Error, sync::Arc, time::Duration};
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
        Arc::new(BlockedDeleter),
        "test".into(),
        Duration::from_millis(50),
        2,
    )?;
    let logs = logs::Logs::default();
    let outcome = service
        .erase("alice", "inline-timeout-001", &Product)
        .with_subscriber(logs.subscriber())
        .await?;
    assert_eq!(outcome.status, ErasureState::Pending);
    assert!(store.is_fenced("alice").await?);
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
