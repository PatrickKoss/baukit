use std::{error::Error, path::PathBuf, sync::Arc};

{% if context.auth_oidc %}use {{ context.app_crate }}_postgres::{PostgresItemRepository, PostgresUserRepository};
{% else %}use {{ context.app_crate }}_postgres::PostgresItemRepository;
{% endif %}
{% if context.auth_oidc %}use {{ context.app_crate }}_services::{ItemService, UserService};
{% else %}use {{ context.app_crate }}_services::ItemService;
{% endif %}
#[tokio::test]
#[ignore = "requires a reachable Docker daemon; run explicitly for PostgreSQL adapter verification"]
async fn postgres_adapter_crud() -> Result<(), Box<dyn Error>> {
    let migrations = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
    let fixture = baukit_test::start_postgres_with_migrations(&migrations).await?;
    let pool = sqlx::PgPool::connect(fixture.connection_url()).await?;
    let repository = Arc::new(PostgresItemRepository::new(pool.clone()));
    let service = ItemService::new(repository.clone());
{% if context.auth_oidc %}    let users = UserService::new(Arc::new(PostgresUserRepository::new(pool.clone())));
{% endif %}
    let created = service.create("postgres item".to_owned()).await?;
    assert_eq!(service.get(created.id).await?, created);
    assert_eq!(service.list().await?, vec![created.clone()]);
    let updated = service.update(created.id, "updated".to_owned()).await?;
    assert_eq!(updated.name, "updated");
    service.delete(created.id).await?;
    assert!(service.list().await?.is_empty());
{% if context.auth_oidc %}
    let first = users.resolve_subject("oidc-subject").await?;
    let second = users.resolve_subject("oidc-subject").await?;
    assert_eq!(first, second);
{% endif %}
    pool.close().await;
    drop(fixture);
    Ok(())
}
