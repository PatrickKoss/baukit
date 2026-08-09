{% if context.worker %}use baukit_jobs::{NewJob, PostgresJobStore};

{% endif %}use sqlx::PgPool;
use uuid::Uuid;

{% if context.auth_oidc %}use {{ context.app_crate }}_domain::{InternalUser, Item};
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}
{% if context.worker %}use {{ context.app_crate }}_domain::{ITEM_CREATED_JOB_TYPE, ItemCreatedJob};
{% endif %}use {{ context.app_crate }}_ports::{ItemRepository, PortFuture, RepositoryError{% if context.auth_oidc %}, UserRepository{% endif %}};

#[derive(Clone)]
pub struct PostgresItemRepository {
    pool: PgPool,
}

impl PostgresItemRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}{% if context.auth_oidc %}

#[derive(Clone)]
pub struct PostgresUserRepository {
    pool: PgPool,
}

impl PostgresUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}{% endif %}

impl ItemRepository for PostgresItemRepository {
    fn list(&self) -> PortFuture<'_, Result<Vec<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, (Uuid, String)>(
                "SELECT id, name FROM items ORDER BY created_at, id",
            )
            .fetch_all(&self.pool)
            .await
            .map(|rows| {
                rows.into_iter()
                    .map(|(id, name)| Item { id, name })
                    .collect()
            })
            .map_err(RepositoryError::unavailable)
        })
    }

    fn get(&self, id: Uuid) -> PortFuture<'_, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, (Uuid, String)>("SELECT id, name FROM items WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await
                .map(|row| row.map(|(id, name)| Item { id, name }))
                .map_err(RepositoryError::unavailable)
        })
    }

    fn create(&self, item: Item) -> PortFuture<'_, Result<Item, RepositoryError>> {
        Box::pin(async move {
{% if context.worker %}            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(RepositoryError::unavailable)?;
            let result = sqlx::query_as::<_, (Uuid, String)>(
                "INSERT INTO items (id, name) VALUES ($1, $2) RETURNING id, name",
            )
            .bind(item.id)
            .bind(&item.name)
            .fetch_one(&mut *transaction)
            .await;
            let (id, name) = match result {
                Ok(row) => row,
                Err(error)
                    if error
                        .as_database_error()
                        .and_then(|error| error.code())
                        .as_deref()
                        == Some("23505") =>
                {
                    return Err(RepositoryError::Conflict);
                }
                Err(error) => return Err(RepositoryError::unavailable(error)),
            };
            let payload = serde_json::to_value(ItemCreatedJob { item_id: id })
                .map_err(RepositoryError::unavailable)?;
            PostgresJobStore::new(self.pool.clone())
                .enqueue_in_transaction(
                    &mut transaction,
                    NewJob::new(ITEM_CREATED_JOB_TYPE, payload, 3)
                        .idempotency_key(format!("item-created:{id}")),
                )
                .await
                .map_err(RepositoryError::unavailable)?;
            transaction
                .commit()
                .await
                .map_err(RepositoryError::unavailable)?;
            Ok(Item { id, name })
{% else %}            let result = sqlx::query_as::<_, (Uuid, String)>(
                "INSERT INTO items (id, name) VALUES ($1, $2) RETURNING id, name",
            )
            .bind(item.id)
            .bind(item.name)
            .fetch_one(&self.pool)
            .await;
            match result {
                Ok((id, name)) => Ok(Item { id, name }),
                Err(error)
                    if error
                        .as_database_error()
                        .and_then(|error| error.code())
                        .as_deref()
                        == Some("23505") =>
                {
                    Err(RepositoryError::Conflict)
                }
                Err(error) => Err(RepositoryError::unavailable(error)),
            }
{% endif %}        })
    }

    fn update(&self, item: Item) -> PortFuture<'_, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, (Uuid, String)>(
                "UPDATE items SET name = $2 WHERE id = $1 RETURNING id, name",
            )
            .bind(item.id)
            .bind(item.name)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.map(|(id, name)| Item { id, name }))
            .map_err(RepositoryError::unavailable)
        })
    }

    fn delete(&self, id: Uuid) -> PortFuture<'_, Result<bool, RepositoryError>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM items WHERE id = $1")
                .bind(id)
                .execute(&self.pool)
                .await
                .map(|result| result.rows_affected() == 1)
                .map_err(RepositoryError::unavailable)
        })
    }

    fn ready(&self) -> PortFuture<'_, Result<(), RepositoryError>> {
        Box::pin(async move {
            self.pool
                .acquire()
                .await
                .map(|_| ())
                .map_err(RepositoryError::unavailable)
        })
    }
}{% if context.auth_oidc %}

impl UserRepository for PostgresUserRepository {
    fn resolve_subject(
        &self,
        subject: String,
    ) -> PortFuture<'_, Result<InternalUser, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, (Uuid, String)>(
                "INSERT INTO user_identities (user_id, subject) VALUES ($1, $2) \
                 ON CONFLICT (subject) DO UPDATE SET subject = EXCLUDED.subject \
                 RETURNING user_id, subject",
            )
            .bind(Uuid::now_v7())
            .bind(subject)
            .fetch_one(&self.pool)
            .await
            .map(|(id, subject)| InternalUser { id, subject })
            .map_err(RepositoryError::unavailable)
        })
    }
}{% endif %}
