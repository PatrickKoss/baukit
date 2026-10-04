use crate::{
    ErasureFuture, IDENTITY_DELETE_JOB_TYPE, IdentityAccountDeleter, IdentityDeletionError,
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use baukit_config::Secret;
use baukit_http::{ApiError, IdempotencyKeyRule};
use baukit_jobs::{NewJob, PostgresJobStore};
use chrono::{DateTime, Utc};
use ring::hmac;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

/// Persistent erasure state.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErasureState {
    /// Product rows are gone; provider deletion is queued.
    Pending,
    /// Provider deletion also finished.
    Completed,
    /// Operators must repair and rerun the retained job.
    Failed,
}
/// Safe response and replay receipt.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ErasureOutcome {
    /// Current operation state.
    pub status: ErasureState,
    /// Opaque operation identifier.
    pub operation_id: Uuid,
    /// Completion timestamp, present only after success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}
impl ErasureOutcome {
    /// HTTP response code for DELETE and its replay receipt.
    pub fn status_code(&self) -> StatusCode {
        match self.status {
            ErasureState::Pending => StatusCode::ACCEPTED,
            ErasureState::Completed | ErasureState::Failed => StatusCode::OK,
        }
    }
}
/// Errors before durable acceptance. Database errors may require client reconciliation.
#[derive(Debug, thiserror::Error)]
pub enum ErasureError {
    /// Required 16..128 visible ASCII key is missing or invalid.
    #[error("invalid erasure idempotency key")]
    InvalidKey,
    /// The key already belongs to another subject.
    #[error("erasure idempotency conflict")]
    Conflict,
    /// A fenced identity cannot start another operation or recreate its profile.
    #[error("profile erased")]
    ProfileErased,
    /// Unsafe or incomplete service configuration.
    #[error("invalid erasure configuration")]
    Configuration,
    /// Database operation failed.
    #[error("erasure database operation failed")]
    Database(#[from] sqlx::Error),
    /// Durable job enqueue failed.
    #[error("erasure job store operation failed")]
    Jobs(#[from] baukit_jobs::StoreError),
    /// Invalid stored response.
    #[error("invalid erasure receipt")]
    Receipt(#[from] serde_json::Error),
}
impl ErasureError {
    fn class(&self) -> &'static str {
        match self {
            Self::InvalidKey => "invalid_key",
            Self::Conflict => "conflict",
            Self::ProfileErased => "profile_erased",
            Self::Configuration => "configuration",
            Self::Database(_) => "database",
            Self::Jobs(_) => "job_store",
            Self::Receipt(_) => "receipt",
        }
    }
}
impl IntoResponse for ErasureError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidKey => ApiError::new(
                StatusCode::BAD_REQUEST,
                "erasure_idempotency_key_invalid",
                "A valid Idempotency-Key is required",
            )
            .into_response(),
            Self::Conflict => ApiError::new(
                StatusCode::CONFLICT,
                "erasure_idempotency_conflict",
                "The erasure key belongs to another identity",
            )
            .into_response(),
            Self::ProfileErased => ApiError::new(
                StatusCode::UNAUTHORIZED,
                "profile_erased",
                "The profile has been erased",
            )
            .into_response(),
            error => ApiError::internal(error).into_response(),
        }
    }
}
/// Product-owned deletion executed inside the erasure transaction.
///
/// Delete every owned database resource, API token, and owned job here. Do not
/// contact external systems or commit the provided connection.
pub trait ProductErasure: Send + Sync {
    /// Deletes the resource graph for one authenticated subject.
    fn erase<'a>(
        &'a self,
        connection: &'a mut PgConnection,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), sqlx::Error>>;
}
/// PostgreSQL receipt and fence store. Products apply the reference migration.
#[derive(Clone)]
pub struct PostgresErasureStore {
    pool: PgPool,
    hash_key: Arc<Secret<String>>,
}
impl PostgresErasureStore {
    /// Requires at least 32 bytes of secret key material. Keep the key stable.
    pub fn new(pool: PgPool, hash_key: Secret<String>) -> Result<Self, ErasureError> {
        if hash_key.expose().len() < 32 {
            return Err(ErasureError::Configuration);
        }
        Ok(Self {
            pool,
            hash_key: Arc::new(hash_key),
        })
    }
    /// Underlying product pool for worker composition.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn hash(&self, domain: &[u8], input: &str) -> Vec<u8> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.hash_key.expose().as_bytes());
        let mut context = hmac::Context::with_key(&key);
        context.update(domain);
        context.update(input.as_bytes());
        context.sign().as_ref().to_vec()
    }
    /// Keyed subject hash, never the provider subject itself.
    pub fn subject_hash(&self, subject: &str) -> Vec<u8> {
        self.hash(b"erasure-subject\0", subject)
    }
    /// Locks subject resolution against erasure and checks its fence in the same transaction.
    pub async fn guard_subject(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> Result<(), ErasureError> {
        let hash = self.subject_hash(subject);
        lock(transaction, &hash).await?;
        let fenced: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM erasure_fences WHERE subject_hash = $1)",
        )
        .bind(hash)
        .fetch_one(&mut **transaction)
        .await?;
        if fenced {
            Err(ErasureError::ProfileErased)
        } else {
            Ok(())
        }
    }
    /// Checks the fence without resolving or creating a profile.
    pub async fn is_fenced(&self, subject: &str) -> Result<bool, ErasureError> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM erasure_fences WHERE subject_hash = $1)",
        )
        .bind(self.subject_hash(subject))
        .fetch_one(&self.pool)
        .await?)
    }
    /// Subject-authorized status lookup. Unknown and foreign operations both return None.
    pub async fn status(
        &self,
        subject: &str,
        operation_id: Uuid,
    ) -> Result<Option<ErasureOutcome>, ErasureError> {
        let response: Option<Value> = sqlx::query_scalar(
            "SELECT response FROM erasure_operations WHERE id = $1 AND subject_hash = $2",
        )
        .bind(operation_id)
        .bind(self.subject_hash(subject))
        .fetch_optional(&self.pool)
        .await?;
        response
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }
    pub(crate) async fn complete(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        operation_id: Uuid,
    ) -> Result<ErasureOutcome, ErasureError> {
        let outcome = ErasureOutcome {
            status: ErasureState::Completed,
            operation_id,
            completed_at: Some(Utc::now()),
        };
        let response: Value = sqlx::query_scalar("UPDATE erasure_operations SET state = 'completed', completed_at = COALESCE(completed_at, $2), response = CASE WHEN state = 'completed' THEN response ELSE $3 END WHERE id = $1 RETURNING response")
            .bind(operation_id).bind(outcome.completed_at).bind(serde_json::to_value(&outcome)?)
            .fetch_one(&mut **transaction).await?;
        Ok(serde_json::from_value(response)?)
    }
}
async fn lock(transaction: &mut Transaction<'_, Postgres>, hash: &[u8]) -> Result<(), sqlx::Error> {
    let mut bytes = [0; 8];
    bytes.copy_from_slice(&hash[..8]);
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(i64::from_be_bytes(bytes))
        .execute(&mut **transaction)
        .await?;
    Ok(())
}
/// Axum handler helper. Call after authentication, before any product operation.
pub async fn reject_fenced_subject(
    store: &PostgresErasureStore,
    subject: &str,
) -> Result<(), ErasureError> {
    if store.is_fenced(subject).await? {
        Err(ErasureError::ProfileErased)
    } else {
        Ok(())
    }
}
/// Coordinates the product transaction, durable job, and bounded inline attempt.
#[derive(Clone)]
pub struct ErasureService {
    store: PostgresErasureStore,
    deleter: Arc<dyn IdentityAccountDeleter>,
    provider_id: String,
    inline_timeout: Duration,
    max_attempts: u32,
}
impl ErasureService {
    /// Builds a service. Provider id must match the registered worker handler.
    /// Keep inline_timeout short: its whole-provider-call budget includes token
    /// acquisition and holds a pooled connection and job row lock until it ends.
    pub fn new(
        store: PostgresErasureStore,
        deleter: Arc<dyn IdentityAccountDeleter>,
        provider_id: String,
        inline_timeout: Duration,
        max_attempts: u32,
    ) -> Result<Self, ErasureError> {
        if provider_id.trim().is_empty() || inline_timeout.is_zero() || max_attempts == 0 {
            return Err(ErasureError::Configuration);
        }
        Ok(Self {
            store,
            deleter,
            provider_id,
            inline_timeout,
            max_attempts,
        })
    }
    /// Receipt and fence store for status handlers and subject resolution.
    pub fn store(&self) -> &PostgresErasureStore {
        &self.store
    }
    /// Erases once under key and subject locks. Replays never call product deletion again.
    pub async fn erase(
        &self,
        subject: &str,
        key: &str,
        product: &dyn ProductErasure,
    ) -> Result<ErasureOutcome, ErasureError> {
        let value = axum::http::HeaderValue::from_str(key).map_err(|_| ErasureError::InvalidKey)?;
        IdempotencyKeyRule::new(16, 128)
            .parse(&value)
            .map_err(|_| ErasureError::InvalidKey)?;
        if subject.is_empty() {
            return Err(ErasureError::Configuration);
        }
        let key_hash = self.store.hash(b"erasure-key\0", key);
        let subject_hash = self.store.subject_hash(subject);
        let mut transaction = self.store.pool.begin().await?;
        lock(&mut transaction, &key_hash).await?;
        let previous: Option<(Vec<u8>, Value)> = sqlx::query_as(
            "SELECT subject_hash, response FROM erasure_operations WHERE key_hash = $1",
        )
        .bind(&key_hash)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some((owner, response)) = previous {
            if owner != subject_hash {
                return Err(ErasureError::Conflict);
            }
            return Ok(serde_json::from_value(response)?);
        }
        self.store.guard_subject(&mut transaction, subject).await?;
        product.erase(&mut transaction, subject).await?;
        let outcome = ErasureOutcome {
            status: ErasureState::Pending,
            operation_id: Uuid::now_v7(),
            completed_at: None,
        };
        sqlx::query("INSERT INTO erasure_operations (id, subject_hash, key_hash, state, response) VALUES ($1, $2, $3, 'pending', $4)")
            .bind(outcome.operation_id).bind(&subject_hash).bind(key_hash).bind(serde_json::to_value(&outcome)?)
            .execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO erasure_fences (subject_hash) VALUES ($1)")
            .bind(subject_hash)
            .execute(&mut *transaction)
            .await?;
        let payload = serde_json::json!({"subject": subject, "providerId": self.provider_id, "operationId": outcome.operation_id});
        let job = PostgresJobStore::new(self.store.pool.clone())
            .enqueue_in_transaction(
                &mut transaction,
                NewJob::new(IDENTITY_DELETE_JOB_TYPE, payload, self.max_attempts)
                    .idempotency_key(outcome.operation_id.to_string()),
            )
            .await?
            .job;
        transaction.commit().await?;
        match self.inline(subject, job.id, outcome.clone()).await {
            Ok(outcome) => Ok(outcome),
            // Acceptance is already durable. The worker also reconciles a provider
            // success whose receipt update failed.
            Err(error) => {
                tracing::warn!(
                    operation_id = %outcome.operation_id,
                    error_class = error.class(),
                    "inline erasure reconciliation failed",
                );
                Ok(outcome)
            }
        }
    }
    async fn inline(
        &self,
        subject: &str,
        job_id: Uuid,
        outcome: ErasureOutcome,
    ) -> Result<ErasureOutcome, ErasureError> {
        let mut transaction = self.store.pool.begin().await?;
        // A worker may already own the job. Only an unclaimed job is completed inline.
        let pending: Option<String> =
            sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1 FOR UPDATE")
                .bind(job_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if pending.as_deref() != Some("pending") {
            transaction.rollback().await?;
            return Ok(self
                .store
                .status(subject, outcome.operation_id)
                .await?
                .unwrap_or(outcome));
        }
        let error_class =
            match tokio::time::timeout(self.inline_timeout, self.deleter.delete_account(subject))
                .await
            {
                Ok(Ok(())) => None,
                Ok(Err(IdentityDeletionError::Retryable)) => Some("retryable"),
                Ok(Err(IdentityDeletionError::Permanent)) => Some("permanent"),
                Err(_) => Some("timeout"),
            };
        if let Some(error_class) = error_class {
            tracing::warn!(
                operation_id = %outcome.operation_id,
                error_class,
                "inline identity deletion failed",
            );
            transaction.rollback().await?;
            return Ok(outcome);
        }
        let completed = self
            .store
            .complete(&mut transaction, outcome.operation_id)
            .await?;
        sqlx::query("DELETE FROM job_outbox WHERE id = $1")
            .bind(job_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(completed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipts_match_shared_wire_vectors() {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct ReceiptVector {
            name: String,
            http_status: u16,
            receipt: Value,
        }
        #[derive(Deserialize)]
        struct Vectors {
            cases: Vec<ReceiptVector>,
        }
        let vectors: Vectors = serde_json::from_str(include_str!(
            "../../../../fixtures/erasure/receipts-v1.json"
        ))
        .expect("shared erasure receipts");
        for vector in vectors.cases {
            let receipt: ErasureOutcome =
                serde_json::from_value(vector.receipt.clone()).expect("receipt");
            assert_eq!(
                receipt.status_code().as_u16(),
                vector.http_status,
                "{}",
                vector.name
            );
            assert_eq!(
                serde_json::to_value(receipt).expect("receipt"),
                vector.receipt,
                "{}",
                vector.name
            );
        }
    }
    #[test]
    fn outcomes_use_contract_status_codes_and_camel_case() {
        let mut outcome = ErasureOutcome {
            status: ErasureState::Pending,
            operation_id: Uuid::nil(),
            completed_at: None,
        };
        assert_eq!(outcome.status_code(), StatusCode::ACCEPTED);
        assert_eq!(
            serde_json::to_value(&outcome).expect("serialize"),
            serde_json::json!({"status":"pending", "operationId":Uuid::nil()})
        );
        outcome.status = ErasureState::Completed;
        outcome.completed_at = Some(Utc::now());
        assert_eq!(outcome.status_code(), StatusCode::OK);
        assert!(serde_json::to_value(outcome).expect("serialize")["completedAt"].is_string());
    }
}
