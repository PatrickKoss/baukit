use std::{error::Error, path::PathBuf, sync::Arc};

use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_services::ItemService;

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for PostgreSQL adapter verification"]
async fn postgres_adapter_crud() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let service = ItemService::new(Arc::new(PostgresItemRepository::new(pool.clone())));

    let created = service.create("postgres item".to_owned()).await?;
    assert_eq!(service.get(created.id).await?, created);
    assert_eq!(service.list().await?, vec![created.clone()]);
    let updated = service.update(created.id, "updated".to_owned()).await?;
    assert_eq!(updated.name, "updated");
    service.delete(created.id).await?;
    assert!(service.list().await?.is_empty());

    pool.close().await;
    drop(fixture);
    Ok(())
}
