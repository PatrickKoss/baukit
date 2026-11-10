use std::{error::Error, path::PathBuf};

use baukit_erasure::ProductErasure as _;
use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use {{ context.app_crate }}_postgres::PostgresProfileErasure;

const ERASED: &str = "erased-subject";
const KEPT: &str = "kept-subject";

#[tokio::test]
#[ignore = "requires Docker PostgreSQL"]
async fn profile_erasure_deletes_only_the_erased_subjects_browser_sessions()
-> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let container = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = PgPool::connect(container.connection_url()).await?;
    let expires_at = Utc::now() + Duration::days(1);
    insert_session(&pool, ERASED, expires_at).await?;
    insert_session(&pool, ERASED, expires_at).await?;
    insert_session(&pool, KEPT, expires_at).await?;

    let mut transaction = pool.begin().await?;
    PostgresProfileErasure
        .erase(&mut transaction, ERASED)
        .await?;
    transaction.commit().await?;

    let subjects: Vec<String> =
        sqlx::query_scalar("SELECT subject FROM baukit_auth_browser_sessions")
            .fetch_all(&pool)
            .await?;
    assert_eq!(subjects, [KEPT]);
    Ok(())
}

async fn insert_session(
    pool: &PgPool,
    subject: &str,
    expires_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO baukit_auth_browser_sessions (
             id, secret_hash, family_id, subject, key_version, refresh_token,
             created_at, last_used_at, idle_expires_at, absolute_expires_at
         ) VALUES ($1, $2, $3, $4, 1, $5, now(), now(), $6, $6)",
    )
    .bind(Uuid::new_v4())
    .bind(vec![0_u8; 32])
    .bind(Uuid::new_v4())
    .bind(subject)
    .bind(vec![0_u8; 16])
    .bind(expires_at)
    .execute(pool)
    .await?;
    Ok(())
}
