use std::{error::Error, path::PathBuf, sync::Arc};

use sqlx::PgPool;
use tokio::sync::Barrier;
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn revisions_are_monotonic_per_owner() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    let mut revisions = Vec::new();
    for _ in 0..5 {
        let mut transaction = pool.begin().await?;
        revisions.push(baukit_sync::next_revision(&mut transaction, owner).await?);
        transaction.commit().await?;
    }

    assert_eq!(revisions, vec![1, 2, 3, 4, 5]);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn owners_have_independent_counters() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let first = owner(&pool).await?;
    let second = owner(&pool).await?;

    let mut transaction = pool.begin().await?;
    baukit_sync::next_revision(&mut transaction, first).await?;
    baukit_sync::next_revision(&mut transaction, first).await?;
    let first_third = baukit_sync::next_revision(&mut transaction, first).await?;
    let second_first = baukit_sync::next_revision(&mut transaction, second).await?;
    transaction.commit().await?;

    assert_eq!(first_third, 3);
    assert_eq!(second_first, 1, "one owner's writes do not advance another");

    let mut transaction = pool.begin().await?;
    assert_eq!(
        baukit_sync::current_revision(&mut transaction, first).await?,
        Some(3)
    );
    assert_eq!(
        baukit_sync::current_revision(&mut transaction, second).await?,
        Some(1)
    );
    transaction.commit().await?;

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_rolled_back_transaction_does_not_consume_a_revision() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    let mut committed = pool.begin().await?;
    let first = baukit_sync::next_revision(&mut committed, owner).await?;
    committed.commit().await?;

    let mut aborted = pool.begin().await?;
    let discarded = baukit_sync::next_revision(&mut aborted, owner).await?;
    aborted.rollback().await?;

    let mut next = pool.begin().await?;
    let reused = baukit_sync::next_revision(&mut next, owner).await?;
    next.commit().await?;

    assert_eq!(first, 1);
    assert_eq!(
        discarded, 2,
        "the aborted transaction saw its own allocation"
    );
    assert_eq!(
        reused, 2,
        "the rollback returned that revision to the owner"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn concurrent_writers_for_one_owner_never_share_a_revision() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    let writers = 8;
    let barrier = Arc::new(Barrier::new(writers));

    let mut handles = Vec::with_capacity(writers);
    for _ in 0..writers {
        let pool = pool.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            barrier.wait().await;
            let mut transaction = pool.begin().await?;
            let revision = baukit_sync::next_revision(&mut transaction, owner).await?;
            transaction.commit().await?;
            Ok::<i64, sqlx::Error>(revision)
        }));
    }

    let mut revisions = Vec::with_capacity(writers);
    for handle in handles {
        revisions.push(handle.await??);
    }
    revisions.sort_unstable();

    assert_eq!(revisions, (1..=writers as i64).collect::<Vec<_>>());

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn locking_reads_serialize_revision_allocations() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;
    let barrier = Arc::new(Barrier::new(2));

    let mut handles = Vec::new();
    for _ in 0..2 {
        let pool = pool.clone();
        let barrier = Arc::clone(&barrier);
        handles.push(tokio::spawn(async move {
            let mut transaction = pool.begin().await?;
            barrier.wait().await;
            let observed =
                baukit_sync::current_revision_for_update(&mut transaction, owner).await?;
            let allocated = baukit_sync::next_revision(&mut transaction, owner).await?;
            transaction.commit().await?;
            Ok::<(Option<i64>, i64), sqlx::Error>((observed, allocated))
        }));
    }

    let mut results = Vec::new();
    for handle in handles {
        results.push(handle.await??);
    }
    results.sort_unstable_by_key(|(_, allocated)| *allocated);

    assert_eq!(results, vec![(Some(0), 1), (Some(1), 2)]);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn rename_sql_upgrades_the_old_counter_shape() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    sqlx::raw_sql(
        "DROP TABLE product_records;
         DROP TABLE sync_revisions;
         CREATE TABLE users (id UUID PRIMARY KEY);
         CREATE TABLE sync_revisions (
             user_id UUID PRIMARY KEY,
             last_revision BIGINT NOT NULL DEFAULT 0,
             CONSTRAINT sync_revisions_user_id_fkey
                 FOREIGN KEY (user_id) REFERENCES users (id) ON DELETE CASCADE
         );",
    )
    .execute(&pool)
    .await?;

    sqlx::raw_sql(baukit_sync::POSTGRES_RENAME_USER_ID_TO_OWNER_ID_SQL)
        .execute(&pool)
        .await?;

    let columns: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT column_name, data_type, is_nullable, column_default
         FROM information_schema.columns
         WHERE table_schema = 'public' AND table_name = 'sync_revisions'
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await?;
    assert_eq!(columns.len(), 2);
    assert_eq!(
        (
            columns[0].0.as_str(),
            columns[0].1.as_str(),
            columns[0].2.as_str()
        ),
        ("owner_id", "uuid", "NO")
    );
    assert_eq!(
        (
            columns[1].0.as_str(),
            columns[1].1.as_str(),
            columns[1].2.as_str()
        ),
        ("last_revision", "bigint", "NO")
    );
    assert!(
        columns[1]
            .3
            .as_deref()
            .is_some_and(|default| default.contains('0'))
    );

    let constraints: Vec<(String, String)> = sqlx::query_as(
        "SELECT constraint_name, constraint_type
         FROM information_schema.table_constraints
         WHERE table_schema = 'public' AND table_name = 'sync_revisions'
         ORDER BY constraint_name",
    )
    .fetch_all(&pool)
    .await?;
    assert!(constraints.contains(&(
        "sync_revisions_owner_id_fkey".to_owned(),
        "FOREIGN KEY".to_owned()
    )));
    assert!(constraints.contains(&(
        "sync_revisions_last_revision_check".to_owned(),
        "CHECK".to_owned()
    )));
    assert!(constraints.contains(&("sync_revisions_pkey".to_owned(), "PRIMARY KEY".to_owned())));

    let delete_rule: String = sqlx::query_scalar(
        "SELECT delete_rule
         FROM information_schema.referential_constraints
         WHERE constraint_schema = 'public'
           AND constraint_name = 'sync_revisions_owner_id_fkey'",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(delete_rule, "CASCADE");

    let owner = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(owner)
        .execute(&pool)
        .await?;
    sqlx::query("INSERT INTO sync_revisions (owner_id) VALUES ($1)")
        .bind(owner)
        .execute(&pool)
        .await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await?;
    let remaining: i64 = sqlx::query_scalar("SELECT count(*) FROM sync_revisions")
        .fetch_one(&pool)
        .await?;
    assert_eq!(remaining, 0);

    let negative_owner = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id) VALUES ($1)")
        .bind(negative_owner)
        .execute(&pool)
        .await?;
    let negative =
        sqlx::query("INSERT INTO sync_revisions (owner_id, last_revision) VALUES ($1, -1)")
            .bind(negative_owner)
            .execute(&pool)
            .await;
    assert!(negative.is_err(), "negative revisions must fail the check");

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn an_unknown_owner_reports_a_missing_counter() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;

    let mut transaction = pool.begin().await?;
    let missing = baukit_sync::next_revision(&mut transaction, Uuid::new_v4()).await;
    transaction.rollback().await?;

    assert!(matches!(missing, Err(sqlx::Error::RowNotFound)));

    let mut transaction = pool.begin().await?;
    let current = baukit_sync::current_revision(&mut transaction, Uuid::new_v4()).await?;
    transaction.commit().await?;
    assert_eq!(current, None);

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn ensure_owner_is_idempotent_and_never_resets_a_counter() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    let mut transaction = pool.begin().await?;
    baukit_sync::next_revision(&mut transaction, owner).await?;
    baukit_sync::next_revision(&mut transaction, owner).await?;
    baukit_sync::ensure_owner(&mut transaction, owner).await?;
    let after = baukit_sync::current_revision(&mut transaction, owner).await?;
    transaction.commit().await?;

    assert_eq!(after, Some(2));

    pool.close().await;
    drop(fixture);
    Ok(())
}

#[tokio::test]
#[ignore = "requires Docker; mandatory in the full local gate"]
async fn a_stamped_row_pulls_back_in_revision_order() -> Result<(), Box<dyn Error>> {
    let (fixture, pool) = fixture().await?;
    let owner = owner(&pool).await?;

    for _ in 0..3 {
        let mut transaction = pool.begin().await?;
        let revision = baukit_sync::next_revision(&mut transaction, owner).await?;
        sqlx::query("INSERT INTO product_records (id, owner_id, revision) VALUES ($1, $2, $3)")
            .bind(Uuid::new_v4())
            .bind(owner)
            .bind(revision)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
    }

    let mut transaction = pool.begin().await?;
    let tombstoned = baukit_sync::next_revision(&mut transaction, owner).await?;
    sqlx::query(
        "UPDATE product_records
         SET deleted_at = now(), updated_at = now(), revision = $1
         WHERE owner_id = $2 AND revision = 1",
    )
    .bind(tombstoned)
    .bind(owner)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;

    let since_revision_2: Vec<(i64, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT revision, deleted_at FROM product_records
         WHERE owner_id = $1 AND revision > 2
         ORDER BY revision",
    )
    .bind(owner)
    .fetch_all(&pool)
    .await?;

    assert_eq!(since_revision_2.len(), 2);
    assert_eq!(since_revision_2[0].0, 3);
    assert_eq!(since_revision_2[1].0, 4);
    assert!(
        since_revision_2[1].1.is_some(),
        "a deletion pulls as a tombstone, not as a missing row"
    );

    pool.close().await;
    drop(fixture);
    Ok(())
}

async fn fixture() -> Result<(baukit_test::PostgresTestContainer, PgPool), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let fixture = baukit_test::start_postgres_with_migrations(migrations).await?;
    let pool = PgPool::connect(fixture.connection_url()).await?;
    sqlx::query(
        "CREATE TABLE product_records (
             id UUID PRIMARY KEY,
             owner_id UUID NOT NULL REFERENCES sync_revisions (owner_id),
             updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
             deleted_at TIMESTAMPTZ,
             revision BIGINT NOT NULL
         )",
    )
    .execute(&pool)
    .await?;
    sqlx::query("CREATE INDEX product_records_sync_idx ON product_records (owner_id, revision)")
        .execute(&pool)
        .await?;
    Ok((fixture, pool))
}

async fn owner(pool: &PgPool) -> Result<Uuid, sqlx::Error> {
    let owner_id = Uuid::new_v4();
    let mut transaction = pool.begin().await?;
    baukit_sync::ensure_owner(&mut transaction, owner_id).await?;
    transaction.commit().await?;
    Ok(owner_id)
}
