#![cfg(test)]
#[path = "support/postgres_database.rs"]
mod postgres_database;
#[path = "support/profiles_product.rs"]
mod profiles_product;

use baukit_config::Secret;
use baukit_erasure::{ErasureError, ErasureService, IdentityRetention, PostgresErasureStore};
use jiff::{SignedDuration, Timestamp};
use std::error::Error;

const OLD_KEY: &str = "test-key-with-at-least-32-bytes-of-entropy";
const NEW_KEY: &str = "rotated-key-with-at-least-32-bytes-of-entropy";
const ERASE_KEY: &str = "rotation-erase-key-01";

fn key(value: &str) -> Secret<String> {
    Secret::new(value.into())
}

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn a_rotated_key_keeps_every_fence_and_receipt() -> Result<(), Box<dyn Error>> {
    let (_container, pool, old_store) = postgres_database::database().await?;
    let login = Timestamp::now()
        .checked_sub(SignedDuration::from_mins(5))
        .ok();
    let erased = ErasureService::new(old_store, IdentityRetention::Retain)?
        .erase("alice", login, ERASE_KEY, &profiles_product::Product)
        .await?;

    let new_only = PostgresErasureStore::new(pool.clone(), key(NEW_KEY))?;
    assert!(
        !new_only.is_fenced("alice", login).await?,
        "without the retired key the fence is invisible"
    );

    let rotating =
        PostgresErasureStore::with_retired_keys(pool.clone(), key(NEW_KEY), vec![key(OLD_KEY)])?;
    assert!(rotating.is_fenced("alice", login).await?);
    assert!(
        rotating
            .status("alice", erased.operation_id)
            .await?
            .is_some()
    );
    let service = ErasureService::new(rotating.clone(), IdentityRetention::Retain)?;
    let replayed = service
        .erase("alice", login, ERASE_KEY, &profiles_product::Product)
        .await?;
    assert_eq!(replayed.operation_id, erased.operation_id);
    assert!(matches!(
        service
            .erase("bob", login, ERASE_KEY, &profiles_product::Product)
            .await,
        Err(ErasureError::Conflict)
    ));

    assert_eq!(rotating.rehash_subjects(&["alice"]).await?, 2);
    assert_eq!(rotating.rehash_subjects(&["alice"]).await?, 0);
    assert!(new_only.is_fenced("alice", login).await?);
    assert!(
        new_only
            .status("alice", erased.operation_id)
            .await?
            .is_some()
    );
    let mut transaction = pool.begin().await?;
    assert!(matches!(
        new_only
            .guard_subject(&mut transaction, "alice", login)
            .await,
        Err(ErasureError::ProfileErased)
    ));
    Ok(())
}

#[tokio::test]
async fn every_key_in_the_ring_needs_32_bytes() {
    let pool = sqlx::PgPool::connect_lazy("postgres://localhost/unused");
    let Ok(pool) = pool else {
        return;
    };
    assert!(matches!(
        PostgresErasureStore::with_retired_keys(pool, key(NEW_KEY), vec![key("short")]),
        Err(ErasureError::Configuration)
    ));
}
