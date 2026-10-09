use baukit_config::Secret;
use baukit_erasure::{ErasureError, ErasureService, IdentityRetention, PostgresErasureStore};
use baukit_test::FakeIdentityAccountDeleter;
use std::{error::Error, sync::Arc, time::Duration};

#[tokio::test]
async fn only_delete_mode_requires_provider_configuration() -> Result<(), Box<dyn Error>> {
    let pool = sqlx::postgres::PgPoolOptions::new().connect_lazy("postgres://localhost/unused")?;
    let store = PostgresErasureStore::new(
        pool,
        Secret::new("test-key-with-at-least-32-bytes-of-entropy".into()),
    )?;
    assert!(ErasureService::new(store.clone(), IdentityRetention::Retain).is_ok());
    for (provider_id, inline_timeout, max_attempts) in [
        ("", Duration::from_secs(1), 2),
        (" \t", Duration::from_secs(1), 2),
        ("test", Duration::ZERO, 2),
        ("test", Duration::from_secs(1), 0),
    ] {
        assert!(matches!(
            ErasureService::new(
                store.clone(),
                IdentityRetention::Delete {
                    deleter: Arc::new(FakeIdentityAccountDeleter::default()),
                    provider_id: provider_id.into(),
                    inline_timeout,
                    max_attempts,
                },
            ),
            Err(ErasureError::Configuration)
        ));
    }
    assert!(
        ErasureService::new(
            store,
            IdentityRetention::Delete {
                deleter: Arc::new(FakeIdentityAccountDeleter::default()),
                provider_id: "test".into(),
                inline_timeout: Duration::from_secs(1),
                max_attempts: 2,
            },
        )
        .is_ok()
    );
    Ok(())
}
