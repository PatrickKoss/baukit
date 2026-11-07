{% if context.platform %}mod platform;
pub use platform::*;

{% endif %}{% if context.worker %}use baukit_jobs::{NewJob, PostgresJobStore};

{% endif %}use sqlx::PgPool;
use uuid::Uuid;

{% if context.auth_enabled %}use {{ context.app_crate }}_domain::InternalUser;
use {{ context.app_crate }}_domain::Item;
{% else %}use {{ context.app_crate }}_domain::Item;
{% endif %}
{% if context.worker %}use {{ context.app_crate }}_domain::ITEM_CREATED_JOB_TYPE;
use {{ context.app_crate }}_domain::ItemCreatedJob;
{% endif %}use {{ context.app_crate }}_ports::ItemRepository;
use {{ context.app_crate }}_ports::PortFuture;
use {{ context.app_crate }}_ports::RepositoryError;
{% if context.auth_enabled %}use {{ context.app_crate }}_ports::UserRepository;
{% endif %}
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
}{% if context.auth_enabled %}

#[derive(Clone)]
pub struct PostgresUserRepository {
    pool: PgPool,
    erasures: baukit_erasure::PostgresErasureStore,
}

impl PostgresUserRepository {
    pub fn new(pool: PgPool, erasures: baukit_erasure::PostgresErasureStore) -> Self {
        Self { pool, erasures }
    }
}{% endif %}

type ItemRow = (Uuid, String, String);

fn item_from_row((id, owner_id, name): ItemRow) -> Item {
    Item { id, owner_id, name }
}

fn is_unique_violation(error: &sqlx::Error) -> bool {
    error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
        == Some("23505")
}

impl ItemRepository for PostgresItemRepository {
    fn list<'a>(&'a self, owner_id: &'a str) -> PortFuture<'a, Result<Vec<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ItemRow>(
                "SELECT id, owner_id, name FROM items WHERE owner_id = $1 ORDER BY created_at, id",
            )
            .bind(owner_id)
            .fetch_all(&self.pool)
            .await
            .map(|rows| rows.into_iter().map(item_from_row).collect())
            .map_err(RepositoryError::unavailable)
        })
    }

    fn get<'a>(
        &'a self,
        owner_id: &'a str,
        id: Uuid,
    ) -> PortFuture<'a, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ItemRow>(
                "SELECT id, owner_id, name FROM items WHERE owner_id = $1 AND id = $2",
            )
            .bind(owner_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.map(item_from_row))
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
            let created = sqlx::query_as::<_, ItemRow>(
                "INSERT INTO items (id, owner_id, name) VALUES ($1, $2, $3) \
                 RETURNING id, owner_id, name",
            )
            .bind(item.id)
            .bind(&item.owner_id)
            .bind(&item.name)
            .fetch_one(&mut *transaction)
            .await
            .map(item_from_row)
            .map_err(|error| {
                if is_unique_violation(&error) {
                    RepositoryError::Conflict
                } else {
                    RepositoryError::unavailable(error)
                }
            })?;
            let payload = serde_json::to_value(ItemCreatedJob {
                item_id: created.id,
            })
            .map_err(RepositoryError::unavailable)?;
            PostgresJobStore::new(self.pool.clone())
                .enqueue_in_transaction(
                    &mut transaction,
                    NewJob::new(ITEM_CREATED_JOB_TYPE, payload, 3)
                        .idempotency_key(format!("item-created:{}", created.id)),
                )
                .await
                .map_err(RepositoryError::unavailable)?;
            transaction
                .commit()
                .await
                .map_err(RepositoryError::unavailable)?;
            Ok(created)
{% else %}            sqlx::query_as::<_, ItemRow>(
                "INSERT INTO items (id, owner_id, name) VALUES ($1, $2, $3) \
                 RETURNING id, owner_id, name",
            )
            .bind(item.id)
            .bind(item.owner_id)
            .bind(item.name)
            .fetch_one(&self.pool)
            .await
            .map(item_from_row)
            .map_err(|error| {
                if is_unique_violation(&error) {
                    RepositoryError::Conflict
                } else {
                    RepositoryError::unavailable(error)
                }
            })
{% endif %}        })
    }

    fn update(&self, item: Item) -> PortFuture<'_, Result<Option<Item>, RepositoryError>> {
        Box::pin(async move {
            sqlx::query_as::<_, ItemRow>(
                "UPDATE items SET name = $3 WHERE owner_id = $1 AND id = $2 \
                 RETURNING id, owner_id, name",
            )
            .bind(item.owner_id)
            .bind(item.id)
            .bind(item.name)
            .fetch_optional(&self.pool)
            .await
            .map(|row| row.map(item_from_row))
            .map_err(RepositoryError::unavailable)
        })
    }

    fn delete<'a>(
        &'a self,
        owner_id: &'a str,
        id: Uuid,
    ) -> PortFuture<'a, Result<bool, RepositoryError>> {
        Box::pin(async move {
            sqlx::query("DELETE FROM items WHERE owner_id = $1 AND id = $2")
                .bind(owner_id)
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
}{% if context.auth_enabled %}

impl UserRepository for PostgresUserRepository {
    fn resolve_subject(
        &self,
        subject: String,
        authenticated_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> PortFuture<'_, Result<InternalUser, RepositoryError>> {
        Box::pin(async move {
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(RepositoryError::unavailable)?;
            self.erasures
                .guard_subject(&mut transaction, &subject, authenticated_at)
                .await
                .map_err(|error| {
                    if matches!(error, baukit_erasure::ErasureError::ProfileErased) {
                        RepositoryError::ProfileErased
                    } else {
                        RepositoryError::unavailable(error)
                    }
                })?;
            let (id, subject) = sqlx::query_as::<_, (Uuid, String)>(
                "INSERT INTO user_identities (user_id, subject) VALUES ($1, $2) \
                 ON CONFLICT (subject) DO UPDATE SET subject = EXCLUDED.subject \
                 RETURNING user_id, subject",
            )
            .bind(Uuid::now_v7())
            .bind(subject)
            .fetch_one(&mut *transaction)
            .await
            .map_err(RepositoryError::unavailable)?;
            transaction
                .commit()
                .await
                .map_err(RepositoryError::unavailable)?;
            Ok(InternalUser { id, subject })
        })
    }
}{% endif %}{% if context.auth_enabled %}

pub struct PostgresProfileErasure;

impl baukit_erasure::ProductErasure for PostgresProfileErasure {
    fn erase<'a>(
        &'a self,
        connection: &'a mut sqlx::PgConnection,
        subject: &'a str,
    ) -> baukit_erasure::ErasureFuture<'a, Result<(), sqlx::Error>> {
        Box::pin(async move {
{% if context.platform %}            baukit_suite::platform::PostgresEntitlementCache::forget_in_transaction(
                subject, connection,
            )
            .await
            .map_err(|error| sqlx::Error::Protocol(error.to_string()))?;
{% endif %}            sqlx::query("DELETE FROM items WHERE owner_id = $1")
                .bind(subject)
                .execute(&mut *connection)
                .await?;
            sqlx::query("DELETE FROM user_identities WHERE subject = $1")
                .bind(subject)
                .execute(connection)
                .await?;
            Ok(())
        })
    }
}{% endif %}
