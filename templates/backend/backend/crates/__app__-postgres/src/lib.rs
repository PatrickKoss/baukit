use {{ context.app_crate }}_domain::Item;
use {{ context.app_crate }}_ports::{ItemRepository, PortFuture, RepositoryError};
use sqlx::PgPool;
use uuid::Uuid;

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
}

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
            let result = sqlx::query_as::<_, (Uuid, String)>(
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
        })
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
}
