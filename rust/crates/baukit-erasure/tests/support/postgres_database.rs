use baukit_config::Secret;
use baukit_erasure::{POSTGRES_MIGRATION_SQL, PostgresErasureStore};
use baukit_test::PostgresTestContainer;
use sqlx::PgPool;
use std::error::Error;

pub async fn database()
-> Result<(PostgresTestContainer, PgPool, PostgresErasureStore), Box<dyn Error>> {
    let container = baukit_test::start_postgres().await?;
    let pool = PgPool::connect(container.connection_url()).await?;
    for migration in [
        baukit_jobs::POSTGRES_MIGRATION_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0002_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0003_SQL,
        baukit_jobs::POSTGRES_MIGRATION_0004_SQL,
        POSTGRES_MIGRATION_SQL,
        baukit_erasure::POSTGRES_REONBOARDING_MIGRATION_SQL,
        baukit_erasure::POSTGRES_OPERATION_ID_GUARD_MIGRATION_SQL,
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
