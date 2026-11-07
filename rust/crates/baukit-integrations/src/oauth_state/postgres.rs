//! PostgreSQL [`OAuthStateStore`]. Apply [`OAUTH_STATE_MIGRATION_SQL`] first.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use super::{
    ConsumeOutcome, OAuthStateError, OAuthStateFuture, OAuthStateStore, PendingOAuthFlow,
    PkceVerifier, TokenHash,
};

/// Tables `oauth_states` and `oauth_states_used`.
pub const OAUTH_STATE_MIGRATION_SQL: &str = include_str!("../../migrations/001_oauth_state.sql");

const CONSUME_SQL: &str = "WITH consumed AS (
    DELETE FROM oauth_states WHERE state_hash = $1
    RETURNING state_hash, binding_hash, user_id, provider, redirect, pkce_verifier, expires_at
), marked AS (
    INSERT INTO oauth_states_used (state_hash, expires_at)
    SELECT state_hash, expires_at FROM consumed
)
SELECT binding_hash, user_id, provider, redirect, pkce_verifier, expires_at FROM consumed";

type ConsumedRow = (
    Vec<u8>,
    String,
    String,
    String,
    Option<String>,
    DateTime<Utc>,
);

/// Pending flows in PostgreSQL. `state_hash` is the key, so queries are owner-agnostic.
#[derive(Clone, Debug)]
pub struct PostgresOAuthStateStore {
    pool: PgPool,
}

impl PostgresOAuthStateStore {
    /// Store on `pool`.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn consume_hash(&self, hash: &TokenHash) -> Result<ConsumeOutcome, sqlx::Error> {
        let row: Option<ConsumedRow> = sqlx::query_as(CONSUME_SQL)
            .bind(hash.0.as_slice())
            .fetch_optional(&self.pool)
            .await?;
        if let Some((binding, user, provider, redirect, verifier, expires_at)) = row {
            let binding_hash = <[u8; 32]>::try_from(binding.as_slice())
                .map(TokenHash)
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
            return Ok(ConsumeOutcome::Consumed(PendingOAuthFlow {
                state_hash: *hash,
                binding_hash,
                user,
                provider,
                redirect,
                pkce_verifier: verifier.map(PkceVerifier::from_stored),
                expires_at,
            }));
        }
        let used: Option<i32> =
            sqlx::query_scalar("SELECT 1 FROM oauth_states_used WHERE state_hash = $1")
                .bind(hash.0.as_slice())
                .fetch_optional(&self.pool)
                .await?;
        Ok(if used.is_some() {
            ConsumeOutcome::AlreadyUsed
        } else {
            ConsumeOutcome::Unknown
        })
    }

    async fn purge(&self, now: DateTime<Utc>) -> Result<u64, sqlx::Error> {
        let pending = sqlx::query("DELETE FROM oauth_states WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        let used = sqlx::query("DELETE FROM oauth_states_used WHERE expires_at <= $1")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(pending.rows_affected().saturating_add(used.rows_affected()))
    }
}

fn store_error(error: &sqlx::Error) -> OAuthStateError {
    OAuthStateError::Store(error.to_string())
}

impl OAuthStateStore for PostgresOAuthStateStore {
    fn insert(&self, flow: PendingOAuthFlow) -> OAuthStateFuture<'_, Result<(), OAuthStateError>> {
        Box::pin(async move {
            sqlx::query(
                "INSERT INTO oauth_states
                 (state_hash, binding_hash, user_id, provider, redirect, pkce_verifier, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(flow.state_hash.0.as_slice())
            .bind(flow.binding_hash.0.as_slice())
            .bind(&flow.user)
            .bind(&flow.provider)
            .bind(&flow.redirect)
            .bind(flow.pkce_verifier.as_ref().map(PkceVerifier::expose))
            .bind(flow.expires_at)
            .execute(&self.pool)
            .await
            .map_err(|error| store_error(&error))?;
            Ok(())
        })
    }

    fn consume<'a>(
        &'a self,
        state_hash: &'a TokenHash,
    ) -> OAuthStateFuture<'a, Result<ConsumeOutcome, OAuthStateError>> {
        Box::pin(async move {
            self.consume_hash(state_hash)
                .await
                .map_err(|error| store_error(&error))
        })
    }

    fn purge_expired(
        &self,
        now: DateTime<Utc>,
    ) -> OAuthStateFuture<'_, Result<u64, OAuthStateError>> {
        Box::pin(async move { self.purge(now).await.map_err(|error| store_error(&error)) })
    }
}
