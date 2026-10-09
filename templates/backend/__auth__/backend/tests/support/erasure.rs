use std::{error::Error, path::PathBuf, sync::Arc, time::Duration};

use baukit_config::Secret;
use baukit_erasure::{ErasureService, IdentityRetention, PostgresErasureStore};
use baukit_test::{FakeIdentityAccountDeleter, PostgresTestContainer};
use sqlx::PgPool;

use {{ context.app_crate }}_api::ErasureApi;
use {{ context.app_crate }}_postgres::PostgresProfileErasure;

pub struct ErasureFixture {
    pub erasure: ErasureApi,
    _container: PostgresTestContainer,
}

pub async fn erasure_fixture() -> Result<ErasureFixture, Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let container = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = PgPool::connect(container.connection_url()).await?;
    let store = PostgresErasureStore::new(
        pool,
        Secret::new("test-hash-key-with-at-least-32-bytes".into()),
    )?;
    Ok(ErasureFixture {
        erasure: ErasureApi {
            service: ErasureService::new(
                store,
                IdentityRetention::Delete {
                    deleter: Arc::new(FakeIdentityAccountDeleter::default()),
                    provider_id: "keycloak".into(),
                    inline_timeout: Duration::from_secs(1),
                    max_attempts: 3,
                },
            )?,
            product: Arc::new(PostgresProfileErasure),
        },
        _container: container,
    })
}
