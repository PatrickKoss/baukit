#[path = "postgres_database.rs"]
mod postgres_database;
#[path = "profiles_product.rs"]
mod profiles_product;

pub use postgres_database::database;
pub use profiles_product::Product;

use baukit_erasure::{ErasureService, IdentityRetention, PostgresErasureStore};
use baukit_test::{FakeIdentityAccountDeleter, PostgresTestContainer};
use sqlx::PgPool;
use std::{error::Error, sync::Arc, time::Duration};

pub async fn fixture() -> Result<
    (
        PostgresTestContainer,
        PgPool,
        PostgresErasureStore,
        Arc<FakeIdentityAccountDeleter>,
        ErasureService,
    ),
    Box<dyn Error>,
> {
    let (container, pool, store) = database().await?;
    let fake = Arc::new(FakeIdentityAccountDeleter::default());
    let service = ErasureService::new(
        store.clone(),
        IdentityRetention::Delete {
            deleter: fake.clone(),
            provider_id: "test".into(),
            inline_timeout: Duration::from_secs(1),
            max_attempts: 2,
        },
    )?;
    Ok((container, pool, store, fake, service))
}
pub async fn count(pool: &PgPool, table: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(match table {
        "profiles" => "SELECT count(*) FROM profiles",
        "erasure_operations" => "SELECT count(*) FROM erasure_operations",
        "erasure_fences" => "SELECT count(*) FROM erasure_fences",
        "job_outbox" => "SELECT count(*) FROM job_outbox",
        _ => panic!("unknown test table"),
    })
    .fetch_one(pool)
    .await
}
