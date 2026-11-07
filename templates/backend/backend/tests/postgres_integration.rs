#[path = "support/owner_scoping.rs"]
mod owner_scoping;

use std::{error::Error, path::PathBuf, sync::Arc};

{% if context.auth_enabled %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
use {{ context.app_crate }}_postgres::PostgresUserRepository;
{% else %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% endif %}
{% if context.auth_enabled %}use {{ context.app_crate }}_services::ItemService;
use {{ context.app_crate }}_services::UserService;
{% else %}use {{ context.app_crate }}_services::ItemService;
{% endif %}
const OWNER: &str = "postgres-owner";

#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for PostgreSQL adapter verification"]
async fn postgres_adapter_crud() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let repository = Arc::new(PostgresItemRepository::new(pool.clone()));
    let service = ItemService::new(repository.clone());
{% if context.auth_enabled %}    let users = UserService::new(Arc::new(PostgresUserRepository::new(
        pool.clone(),
        baukit_erasure::PostgresErasureStore::new(
            pool.clone(),
            baukit_config::Secret::new("test-hash-key-with-at-least-32-bytes".into()),
        )?,
    )));
{% endif %}
    let created = service.create(OWNER, "postgres item".to_owned()).await?;
    assert_eq!(service.get(OWNER, created.id).await?, created);
    assert_eq!(service.list(OWNER).await?, vec![created.clone()]);
    let updated = service
        .update(OWNER, created.id, "updated".to_owned())
        .await?;
    assert_eq!(updated.name, "updated");
    service.delete(OWNER, created.id).await?;
    assert!(service.list(OWNER).await?.is_empty());
    owner_scoping::assert_items_are_owner_scoped(repository.as_ref()).await?;
{% if context.auth_enabled %}
    let first = users.resolve_subject("oidc-subject", None).await?;
    let second = users.resolve_subject("oidc-subject", None).await?;
    assert_eq!(first, second);
{% endif %}
    pool.close().await;
    drop(fixture);
    Ok(())
}
