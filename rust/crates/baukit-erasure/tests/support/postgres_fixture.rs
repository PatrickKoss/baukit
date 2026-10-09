use baukit_config::Secret;
use baukit_erasure::{
    ErasureFuture, ErasureService, IdentityRetention, POSTGRES_MIGRATION_SQL, PostgresErasureStore,
    ProductErasure,
};
use baukit_test::{FakeIdentityAccountDeleter, PostgresTestContainer};
use sqlx::{PgConnection, PgPool};
use std::{error::Error, sync::Arc, time::Duration};

pub struct Product;
impl ProductErasure for Product {
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM profiles WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}
pub async fn database()
-> Result<(PostgresTestContainer, PgPool, PostgresErasureStore), Box<dyn Error>> {
    let container = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(container.connection_url()).await?;
    for migration in [
        baukit_jobs::POSTGRES_MIGRATION_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
        POSTGRES_MIGRATION_SQL,
        "CREATE TABLE profiles (subject TEXT PRIMARY KEY); INSERT INTO profiles VALUES ('alice'), ('bob');",
    ] {
        sqlx::raw_sql(migration).execute(&pool).await?;
    }
    let store = PostgresErasureStore::new(
        pool.clone(),
        Secret::new("test-key-with-at-least-32-bytes-of-entropy".into()),
    )?;
    Ok((container, pool, store))
}
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
