use std::{collections::BTreeSet, num::NonZeroU32};

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgExecutor, PgPool, Row as _, postgres::PgRow};
use uuid::Uuid;

use crate::{
    ApiToken, ApiTokenPolicyRejection, ApiTokenRecord, ApiTokenStore, ApiTokenStoreError,
    ApiTokenStoreFuture, StoredApiToken,
};

macro_rules! token_columns {
    () => {
        "id, owner_id, name, token_prefix, grants, created_at, expires_at, last_used_at, revoked_at"
    };
}

const OWNER_LOCK_NAMESPACE: &str = "baukit_auth.api_tokens:";

/// PostgreSQL [`ApiTokenStore`] over the table in
/// [`POSTGRES_API_TOKENS_MIGRATION_SQL`](crate::POSTGRES_API_TOKENS_MIGRATION_SQL).
///
/// Grants are written in the same `INSERT` as the digest, so a token and its
/// grants commit together or not at all. `touch_last_used` only moves
/// `last_used_at` forward, so concurrent requests keep the latest use.
#[derive(Clone, Debug)]
pub struct PostgresApiTokenStore {
    pool: PgPool,
    active_limit: Option<ActiveTokenLimit>,
}

#[derive(Clone, Debug)]
struct ActiveTokenLimit {
    maximum: NonZeroU32,
    rejection: ApiTokenPolicyRejection,
}

impl PostgresApiTokenStore {
    /// Creates a store without a per-owner token limit.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            active_limit: None,
        }
    }

    /// Rejects issuing when the owner already has `maximum` active tokens.
    ///
    /// Active means unrevoked and unexpired at the new token's `created_at`.
    /// The count and the insert run in one transaction under a per-owner
    /// advisory lock, so concurrent issues cannot overshoot the limit. Over
    /// the limit, `create` returns `rejection` as
    /// [`ApiTokenStoreError::PolicyRejected`] and writes nothing.
    #[must_use]
    pub fn with_active_token_limit(
        mut self,
        maximum: NonZeroU32,
        rejection: ApiTokenPolicyRejection,
    ) -> Self {
        self.active_limit = Some(ActiveTokenLimit { maximum, rejection });
        self
    }

    async fn create_token(&self, record: ApiTokenRecord) -> Result<ApiToken, ApiTokenStoreError> {
        let Some(limit) = &self.active_limit else {
            return insert_token(&self.pool, &record)
                .await
                .map_err(ApiTokenStoreError::internal);
        };
        let mut transaction = self
            .pool
            .begin()
            .await
            .map_err(ApiTokenStoreError::internal)?;
        let active = lock_and_count_active(&mut transaction, record.owner_id, record.created_at)
            .await
            .map_err(ApiTokenStoreError::internal)?;
        if active >= i64::from(limit.maximum.get()) {
            return Err(ApiTokenStoreError::PolicyRejected(limit.rejection.clone()));
        }
        let token = insert_token(&mut *transaction, &record)
            .await
            .map_err(ApiTokenStoreError::internal)?;
        transaction
            .commit()
            .await
            .map_err(ApiTokenStoreError::internal)?;
        Ok(token)
    }

    async fn find_token(
        &self,
        secret_hash: &[u8],
    ) -> Result<Option<StoredApiToken>, ApiTokenStoreError> {
        let row = sqlx::query(concat!(
            "SELECT ",
            token_columns!(),
            ", token_hash FROM api_tokens WHERE token_hash = $1"
        ))
        .bind(secret_hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(ApiTokenStoreError::internal)?;
        row.map(|row| stored_token(&row))
            .transpose()
            .map_err(ApiTokenStoreError::internal)
    }

    async fn touch(
        &self,
        token_id: Uuid,
        used_at: DateTime<Utc>,
    ) -> Result<(), ApiTokenStoreError> {
        sqlx::query(
            "UPDATE api_tokens SET last_used_at = $2
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < $2)",
        )
        .bind(token_id)
        .bind(used_at)
        .execute(&self.pool)
        .await
        .map_err(ApiTokenStoreError::internal)?;
        Ok(())
    }

    async fn revoke_token(
        &self,
        owner_id: Uuid,
        token_id: Uuid,
        revoked_at: DateTime<Utc>,
    ) -> Result<bool, ApiTokenStoreError> {
        let revoked = sqlx::query(
            "UPDATE api_tokens SET revoked_at = $3
             WHERE owner_id = $1 AND id = $2 AND revoked_at IS NULL",
        )
        .bind(owner_id)
        .bind(token_id)
        .bind(revoked_at)
        .execute(&self.pool)
        .await
        .map_err(ApiTokenStoreError::internal)?
        .rows_affected();
        Ok(revoked == 1)
    }

    async fn list_tokens(&self, owner_id: Uuid) -> Result<Vec<ApiToken>, ApiTokenStoreError> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            token_columns!(),
            " FROM api_tokens WHERE owner_id = $1 ORDER BY created_at DESC, id DESC"
        ))
        .bind(owner_id)
        .fetch_all(&self.pool)
        .await
        .map_err(ApiTokenStoreError::internal)?;
        rows.iter()
            .map(api_token)
            .collect::<Result<_, _>>()
            .map_err(ApiTokenStoreError::internal)
    }
}

impl ApiTokenStore for PostgresApiTokenStore {
    fn create(
        &self,
        record: ApiTokenRecord,
    ) -> ApiTokenStoreFuture<'_, Result<ApiToken, ApiTokenStoreError>> {
        Box::pin(self.create_token(record))
    }

    fn find_by_hash<'a>(
        &'a self,
        secret_hash: &'a [u8],
    ) -> ApiTokenStoreFuture<'a, Result<Option<StoredApiToken>, ApiTokenStoreError>> {
        Box::pin(self.find_token(secret_hash))
    }

    fn touch_last_used(
        &self,
        token_id: Uuid,
        used_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<(), ApiTokenStoreError>> {
        Box::pin(self.touch(token_id, used_at))
    }

    fn revoke(
        &self,
        owner_id: Uuid,
        token_id: Uuid,
        revoked_at: DateTime<Utc>,
    ) -> ApiTokenStoreFuture<'_, Result<bool, ApiTokenStoreError>> {
        Box::pin(self.revoke_token(owner_id, token_id, revoked_at))
    }

    fn list_for_owner(
        &self,
        owner_id: Uuid,
    ) -> ApiTokenStoreFuture<'_, Result<Vec<ApiToken>, ApiTokenStoreError>> {
        Box::pin(self.list_tokens(owner_id))
    }
}

/// Deletes every token belonging to one owner and returns how many rows went.
///
/// Products whose owner foreign key uses `ON DELETE CASCADE` get this for free.
/// Call it inside the product's erasure transaction when the owner row is kept
/// or no foreign key exists.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn erase_owner_api_tokens<'e, E>(executor: E, owner_id: Uuid) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query("DELETE FROM api_tokens WHERE owner_id = $1")
        .bind(owner_id)
        .execute(executor)
        .await?
        .rows_affected();
    Ok(deleted)
}

/// Deletes one batch of tokens revoked or expired before `cutoff`.
///
/// Returns the number of deleted rows. Call it again until it returns less
/// than `limit`. Rows locked by a concurrent writer are skipped, not waited on.
///
/// # Errors
///
/// Returns any database error unchanged.
pub async fn purge_inactive_api_tokens<'e, E>(
    executor: E,
    cutoff: DateTime<Utc>,
    limit: NonZeroU32,
) -> Result<u64, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let deleted = sqlx::query(
        "DELETE FROM api_tokens WHERE id IN (
             SELECT id FROM api_tokens
             WHERE revoked_at < $1 OR expires_at < $1
             ORDER BY id
             LIMIT $2
             FOR UPDATE SKIP LOCKED
         )",
    )
    .bind(cutoff)
    .bind(i64::from(limit.get()))
    .execute(executor)
    .await?
    .rows_affected();
    Ok(deleted)
}

async fn lock_and_count_active(
    connection: &mut PgConnection,
    owner_id: Uuid,
    at: DateTime<Utc>,
) -> Result<i64, sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1 || $2::text, 0))")
        .bind(OWNER_LOCK_NAMESPACE)
        .bind(owner_id)
        .execute(&mut *connection)
        .await?;
    sqlx::query_scalar(
        "SELECT count(*) FROM api_tokens
         WHERE owner_id = $1
           AND revoked_at IS NULL
           AND (expires_at IS NULL OR expires_at > $2)",
    )
    .bind(owner_id)
    .bind(at)
    .fetch_one(connection)
    .await
}

async fn insert_token<'e, E>(executor: E, record: &ApiTokenRecord) -> Result<ApiToken, sqlx::Error>
where
    E: PgExecutor<'e>,
{
    let row = sqlx::query(concat!(
        "INSERT INTO api_tokens
             (id, owner_id, name, token_hash, token_prefix, grants, created_at, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING ",
        token_columns!()
    ))
    .bind(record.id)
    .bind(record.owner_id)
    .bind(&record.name)
    .bind(&record.secret_hash)
    .bind(&record.display_prefix)
    .bind(record.grants.iter().map(String::as_str).collect::<Vec<_>>())
    .bind(record.created_at)
    .bind(record.expires_at)
    .fetch_one(executor)
    .await?;
    api_token(&row)
}

fn stored_token(row: &PgRow) -> Result<StoredApiToken, sqlx::Error> {
    Ok(StoredApiToken {
        token: api_token(row)?,
        secret_hash: row.try_get("token_hash")?,
    })
}

fn api_token(row: &PgRow) -> Result<ApiToken, sqlx::Error> {
    let grants: Vec<String> = row.try_get("grants")?;
    Ok(ApiToken {
        id: row.try_get("id")?,
        owner_id: row.try_get("owner_id")?,
        name: row.try_get("name")?,
        display_prefix: row.try_get("token_prefix")?,
        grants: grants.into_iter().collect::<BTreeSet<_>>(),
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        last_used_at: row.try_get("last_used_at")?,
        revoked_at: row.try_get("revoked_at")?,
    })
}
