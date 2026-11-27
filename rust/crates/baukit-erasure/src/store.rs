use crate::kernel::{ReplayDecision, replay_decision};
use crate::{
    ErasureFuture, IDENTITY_DELETE_JOB_TYPE, IdentityAccountDeleter, IdentityDeletionError,
};
use aws_lc_rs::hmac;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use baukit_config::Secret;
use baukit_http::{ApiError, IdempotencyKeyRule};
use baukit_jobs::{NewJob, PostgresJobStore};
use jiff::{SignedDuration, Timestamp};
use jiff_sqlx::ToSqlx;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

/// Time past the inline timeout before a worker may claim the identity job.
const INLINE_RESERVATION_MARGIN: Duration = Duration::from_secs(5);
const MIN_HASH_KEY_BYTES: usize = 32;
const SUBJECT_DOMAIN: &[u8] = b"erasure-subject\0";
const KEY_DOMAIN: &[u8] = b"erasure-key\0";

/// Mirrored by [`crate::kernel::fence_rejects`]; keep both in step. `$1` is every hash of
/// the subject, one per configured key.
pub const REJECTED_SESSION_SQL: &str = "SELECT EXISTS (
    SELECT 1 FROM erasure_fences AS fence
    WHERE fence.subject_hash = ANY($1) AND (
        $2::timestamptz IS NULL OR $2 <= fence.erased_at OR EXISTS (
            SELECT 1 FROM erasure_operations AS operation
            WHERE operation.subject_hash = ANY($1) AND operation.state <> 'completed'
        )
    )
)";

/// Persistent erasure state.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ErasureState {
    /// Product rows are gone; provider deletion is queued.
    Pending,
    /// Product erasure and any requested provider deletion finished.
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
    /// Opaque operation identifier, required on every wire receipt.
    pub operation_id: Uuid,
    /// Completion timestamp, present only after success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<Timestamp>,
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
/// Read-only admission result before external erasure notifications.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErasurePreflight {
    /// The request passed validation and can attempt erasure.
    Proceed,
    /// The key already has a receipt for this subject. Do not notify again.
    Replay(ErasureOutcome),
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
    /// This session cannot start another operation or recreate its profile after erasure.
    #[error("profile erased")]
    ProfileErased,
    /// Unsafe or incomplete service configuration.
    #[error("invalid erasure configuration")]
    Configuration,
    /// The subject is not one well-formed provider path segment, see
    /// [`crate::kernel::is_well_formed_subject`]. Nothing was written or queued.
    #[error("invalid erasure subject")]
    InvalidSubject,
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
            Self::InvalidSubject => "invalid_subject",
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
    /// Keys from before a rotation, read until every row moved to `hash_key`.
    retired_keys: Arc<Vec<Secret<String>>>,
}
impl PostgresErasureStore {
    /// Requires at least 32 bytes of secret key material.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Configuration`] when `hash_key` is shorter than 32 bytes.
    pub fn new(pool: PgPool, hash_key: Secret<String>) -> Result<Self, ErasureError> {
        Self::with_retired_keys(pool, hash_key, Vec::new())
    }
    /// Rotates to `hash_key` while rows hashed with `retired` keys still match.
    ///
    /// New rows use `hash_key`. Every locked access to a subject rewrites its rows to
    /// `hash_key`, and [`Self::rehash_subjects`] does the same for a list of subjects. Drop a
    /// retired key only after every erased subject was rehashed, or its fence stops matching.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Configuration`] when any key is shorter than 32 bytes.
    pub fn with_retired_keys(
        pool: PgPool,
        hash_key: Secret<String>,
        retired: Vec<Secret<String>>,
    ) -> Result<Self, ErasureError> {
        if std::iter::once(&hash_key)
            .chain(&retired)
            .any(|key| key.expose().len() < MIN_HASH_KEY_BYTES)
        {
            return Err(ErasureError::Configuration);
        }
        Ok(Self {
            pool,
            hash_key: Arc::new(hash_key),
            retired_keys: Arc::new(retired),
        })
    }
    /// Underlying product pool for worker composition.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
    fn hash(&self, domain: &[u8], input: &str) -> Vec<u8> {
        keyed_hash(&self.hash_key, domain, input)
    }
    fn retired_hashes(&self, domain: &[u8], input: &str) -> Vec<Vec<u8>> {
        self.retired_keys
            .iter()
            .map(|key| keyed_hash(key, domain, input))
            .collect()
    }
    /// The current hash first, then one per retired key.
    fn hashes(&self, domain: &[u8], input: &str) -> Vec<Vec<u8>> {
        std::iter::once(self.hash(domain, input))
            .chain(self.retired_hashes(domain, input))
            .collect()
    }
    /// Keyed subject hash under the current key, never the provider subject itself.
    pub fn subject_hash(&self, subject: &str) -> Vec<u8> {
        self.hash(SUBJECT_DOMAIN, subject)
    }
    fn subject_hashes(&self, subject: &str) -> Vec<Vec<u8>> {
        self.hashes(SUBJECT_DOMAIN, subject)
    }
    /// Locks `subject` and moves its fence and operations from retired keys to the current
    /// key. Call it while still holding retired keys, then drop them.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Database`] when a lock, query or commit fails.
    pub async fn rehash_subjects(&self, subjects: &[&str]) -> Result<u64, ErasureError> {
        let mut moved = 0;
        for subject in subjects {
            let mut transaction = self.pool.begin().await?;
            moved += self.lock_subject(&mut transaction, subject).await?;
            transaction.commit().await?;
        }
        Ok(moved)
    }
    /// Locks every hash of `subject` and rewrites retired-key rows to the current key.
    async fn lock_subject(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> Result<u64, ErasureError> {
        lock_all(transaction, &self.subject_hashes(subject)).await?;
        let current = self.subject_hash(subject);
        let hashes = self.retired_hashes(SUBJECT_DOMAIN, subject);
        if hashes.is_empty() {
            return Ok(0);
        }
        let fences = sqlx::query(
            "WITH retired AS (DELETE FROM erasure_fences WHERE subject_hash = ANY($2) RETURNING erased_at)
             INSERT INTO erasure_fences (subject_hash, erased_at)
             SELECT $1, max(erased_at) FROM retired HAVING count(*) > 0
             ON CONFLICT (subject_hash)
             DO UPDATE SET erased_at = GREATEST(erasure_fences.erased_at, EXCLUDED.erased_at)",
        )
        .bind(&current)
        .bind(&hashes)
        .execute(&mut **transaction)
        .await?
        .rows_affected();
        let operations = sqlx::query(
            "UPDATE erasure_operations SET subject_hash = $1 WHERE subject_hash = ANY($2)",
        )
        .bind(&current)
        .bind(&hashes)
        .execute(&mut **transaction)
        .await?
        .rows_affected();
        Ok(fences + operations)
    }
    /// Locks subject resolution against erasure and checks this session in the same transaction.
    /// Admission requires a login after the fence and no unfinished erasure operations.
    ///
    /// # Errors
    ///
    /// [`ErasureError::ProfileErased`] when the session is fenced, and
    /// [`ErasureError::Database`] when locking or the fence query fails.
    pub async fn guard_subject(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        subject: &str,
        authenticated_at: Option<Timestamp>,
    ) -> Result<(), ErasureError> {
        self.lock_subject(transaction, subject).await?;
        let fenced: bool = sqlx::query_scalar(REJECTED_SESSION_SQL)
            .bind(self.subject_hashes(subject))
            .bind(authenticated_at.map(ToSqlx::to_sqlx))
            .fetch_one(&mut **transaction)
            .await?;
        if fenced {
            Err(ErasureError::ProfileErased)
        } else {
            Ok(())
        }
    }
    /// Begins a product write transaction that already holds the subject lock and passed the
    /// fence. Run every product write for an authenticated subject through it: an erase
    /// that commits first rejects the write, and an erase that starts later waits for the
    /// write to commit and then deletes its rows.
    ///
    /// # Errors
    ///
    /// [`ErasureError::ProfileErased`] when the session is fenced, and
    /// [`ErasureError::Database`] when the transaction, lock or fence query fails.
    pub async fn begin_guarded_write(
        &self,
        subject: &str,
        authenticated_at: Option<Timestamp>,
    ) -> Result<Transaction<'static, Postgres>, ErasureError> {
        let mut transaction = self.pool.begin().await?;
        self.guard_subject(&mut transaction, subject, authenticated_at)
            .await?;
        Ok(transaction)
    }
    /// Returns whether this session is rejected, without creating a profile.
    /// This read takes no lock, so it cannot protect a later write. Use
    /// [`Self::begin_guarded_write`] or [`Self::guard_subject`] for writes.
    /// A missing login time keeps a subject fenced permanently.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Database`] when the fence query fails.
    pub async fn is_fenced(
        &self,
        subject: &str,
        authenticated_at: Option<Timestamp>,
    ) -> Result<bool, ErasureError> {
        Ok(sqlx::query_scalar(REJECTED_SESSION_SQL)
            .bind(self.subject_hashes(subject))
            .bind(authenticated_at.map(ToSqlx::to_sqlx))
            .fetch_one(&self.pool)
            .await?)
    }
    /// Subject-authorized status lookup. Unknown and foreign operations both return None.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Database`] when the query fails and [`ErasureError::Receipt`] when the
    /// stored response does not decode.
    pub async fn status(
        &self,
        subject: &str,
        operation_id: Uuid,
    ) -> Result<Option<ErasureOutcome>, ErasureError> {
        let response: Option<Value> = sqlx::query_scalar(
            "SELECT response FROM erasure_operations WHERE id = $1 AND subject_hash = ANY($2)",
        )
        .bind(operation_id)
        .bind(self.subject_hashes(subject))
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
            completed_at: Some(Timestamp::now()),
        };
        let response: Value = sqlx::query_scalar("UPDATE erasure_operations SET state = 'completed', completed_at = COALESCE(completed_at, $2), response = CASE WHEN state = 'completed' THEN response ELSE $3 END WHERE id = $1 RETURNING response")
            .bind(operation_id).bind(outcome.completed_at.map(ToSqlx::to_sqlx)).bind(serde_json::to_value(&outcome)?)
            .fetch_one(&mut **transaction).await?;
        Ok(serde_json::from_value(response)?)
    }
    pub(crate) async fn fail(&self, operation_id: Uuid) -> Result<bool, ErasureError> {
        let failed = sqlx::query("UPDATE erasure_operations SET state = 'failed', response = jsonb_build_object('status', 'failed', 'operationId', id) WHERE id = $1 AND state = 'pending'")
            .bind(operation_id)
            .execute(&self.pool)
            .await?;
        Ok(failed.rows_affected() > 0)
    }
}
fn keyed_hash(secret: &Secret<String>, domain: &[u8], input: &str) -> Vec<u8> {
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret.expose().as_bytes());
    let mut context = hmac::Context::with_key(&key);
    context.update(domain);
    context.update(input.as_bytes());
    context.sign().as_ref().to_vec()
}
/// Takes one advisory lock per hash in ascending key order, so processes on old and new key
/// rings still exclude each other during a rotation without deadlocking.
async fn lock_all(
    transaction: &mut Transaction<'_, Postgres>,
    hashes: &[Vec<u8>],
) -> Result<(), sqlx::Error> {
    let mut keys = hashes
        .iter()
        .map(|hash| {
            hash.first_chunk::<8>()
                .map(|bytes| i64::from_be_bytes(*bytes))
                .ok_or_else(|| {
                    sqlx::Error::Protocol("advisory lock hash is shorter than 8 bytes".into())
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    keys.sort_unstable();
    keys.dedup();
    for key in keys {
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(key)
            .execute(&mut **transaction)
            .await?;
    }
    Ok(())
}
/// Axum handler helper. Call after authentication to reject fenced sessions early.
///
/// The check takes no lock: an erase can commit between this check and a later write.
/// Product writes must still run inside [`PostgresErasureStore::begin_guarded_write`] or
/// call [`PostgresErasureStore::guard_subject`] in their own transaction.
///
/// # Errors
///
/// [`ErasureError::ProfileErased`] when the session is fenced, and
/// [`ErasureError::Database`] when the fence query fails.
pub async fn reject_fenced_subject(
    store: &PostgresErasureStore,
    subject: &str,
    authenticated_at: Option<Timestamp>,
) -> Result<(), ErasureError> {
    if store.is_fenced(subject, authenticated_at).await? {
        Err(ErasureError::ProfileErased)
    } else {
        Ok(())
    }
}
/// Whether product erasure also deletes the identity provider account.
#[derive(Clone)]
pub enum IdentityRetention {
    /// Deletes the provider account through a durable job and bounded inline attempt.
    Delete {
        /// Provider-neutral account deletion implementation.
        deleter: Arc<dyn IdentityAccountDeleter>,
        /// Provider id matching the registered worker handler.
        provider_id: String,
        /// Whole-provider-call budget, including token acquisition.
        /// No database connection or lock is held during the call. Workers leave the
        /// queued job alone until this budget plus a 5 s margin has passed, so keep it short.
        inline_timeout: Duration,
        /// Maximum durable worker attempts.
        max_attempts: u32,
    },
    /// Keeps the provider account and sessions. A fresh login can re-onboard after completion.
    Retain,
}
/// Coordinates product erasure and the chosen identity retention mode.
#[derive(Clone)]
pub struct ErasureService {
    store: PostgresErasureStore,
    identity_retention: IdentityRetention,
}
impl ErasureService {
    /// Builds a service with an explicit identity retention mode.
    /// Delete mode requires a nonempty provider id, nonzero timeout and attempts.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Configuration`] in delete mode with a blank provider id, zero inline
    /// timeout, or zero max attempts.
    pub fn new(
        store: PostgresErasureStore,
        identity_retention: IdentityRetention,
    ) -> Result<Self, ErasureError> {
        if let IdentityRetention::Delete {
            provider_id,
            inline_timeout,
            max_attempts,
            ..
        } = &identity_retention
            && (provider_id.trim().is_empty() || inline_timeout.is_zero() || *max_attempts == 0)
        {
            return Err(ErasureError::Configuration);
        }
        Ok(Self {
            store,
            identity_retention,
        })
    }
    /// Receipt and fence store for status handlers and subject resolution.
    pub fn store(&self) -> &PostgresErasureStore {
        &self.store
    }
    /// Readiness check for the identity provider in delete mode. Register it with the
    /// readiness registry so a wrong base URL keeps the service unready.
    ///
    /// # Errors
    ///
    /// The deleter's [`IdentityDeletionError`] when its endpoint does not answer like the
    /// provider. Retain mode always succeeds.
    pub async fn probe_identity_provider(&self) -> Result<(), IdentityDeletionError> {
        match &self.identity_retention {
            IdentityRetention::Delete { deleter, .. } => deleter.probe().await,
            IdentityRetention::Retain => Ok(()),
        }
    }
    /// Fences an operator-selected subject inside the caller's transaction.
    ///
    /// The caller erases product rows in this same transaction and commits it.
    /// Retain mode completes immediately. Delete mode queues identity deletion;
    /// register `IdentityDeletionHandler` to process it. No provider call runs here.
    /// Existing fences do not prevent an authorized operator from erasing again.
    ///
    /// # Errors
    ///
    /// [`ErasureError::Configuration`] for an empty subject, [`ErasureError::InvalidSubject`]
    /// for any other subject the kernel rejects, [`ErasureError::Database`] or
    /// [`ErasureError::Jobs`] when writing the fence or queueing identity deletion fails, and
    /// [`ErasureError::Receipt`] when the outcome cannot be serialized or decoded.
    pub async fn fence_as_operator_in_transaction(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        subject: &str,
    ) -> Result<ErasureOutcome, ErasureError> {
        validate_subject(subject)?;
        self.store.lock_subject(transaction, subject).await?;
        let subject_hash = self.store.subject_hash(subject);
        let outcome = ErasureOutcome {
            status: ErasureState::Pending,
            operation_id: Uuid::now_v7(),
            completed_at: None,
        };
        let key_hash = self
            .store
            .hash(b"erasure-operator-key\0", &outcome.operation_id.to_string());
        sqlx::query("INSERT INTO erasure_operations (id, subject_hash, key_hash, state, response) VALUES ($1, $2, $3, 'pending', $4)")
            .bind(outcome.operation_id).bind(&subject_hash).bind(key_hash)
            .bind(serde_json::to_value(&outcome)?)
            .execute(&mut **transaction).await?;
        sqlx::query("INSERT INTO erasure_fences (subject_hash, erased_at) VALUES ($1, now()) ON CONFLICT (subject_hash) DO UPDATE SET erased_at = now()")
            .bind(subject_hash).execute(&mut **transaction).await?;
        if let IdentityRetention::Delete {
            provider_id,
            max_attempts,
            ..
        } = &self.identity_retention
        {
            PostgresJobStore::new(self.store.pool.clone()).enqueue_in_transaction(
                transaction,
                NewJob::new(
                    IDENTITY_DELETE_JOB_TYPE,
                    serde_json::json!({"subject": subject, "providerId": provider_id, "operationId": outcome.operation_id}),
                    *max_attempts,
                ).idempotency_key(outcome.operation_id.to_string()),
            ).await?;
            Ok(outcome)
        } else {
            self.store.complete(transaction, outcome.operation_id).await
        }
    }
    /// Checks the key, subject, replay receipt and session fence without writes or locks.
    ///
    /// Use this before external notifications. `erase` repeats every check under
    /// its locks because this result does not reserve the key or subject.
    ///
    /// # Errors
    ///
    /// [`ErasureError::InvalidKey`] for a key that is not a 16 to 128 byte header value,
    /// [`ErasureError::Configuration`] for an empty subject, [`ErasureError::InvalidSubject`] for
    /// any other subject the kernel rejects, [`ErasureError::Conflict`] when the
    /// key belongs to another subject, [`ErasureError::ProfileErased`] for a fenced session, and
    /// [`ErasureError::Database`] or [`ErasureError::Receipt`] when the lookup fails or the
    /// stored response does not decode.
    pub async fn preflight(
        &self,
        subject: &str,
        authenticated_at: Option<Timestamp>,
        key: &str,
    ) -> Result<ErasurePreflight, ErasureError> {
        validate_request(subject, key)?;
        let previous = sqlx::query_as(
            "SELECT subject_hash, response FROM erasure_operations WHERE key_hash = ANY($1)",
        )
        .bind(self.store.hashes(KEY_DOMAIN, key))
        .fetch_optional(&self.store.pool)
        .await?;
        if let Some(outcome) = replay_outcome(&self.store.subject_hashes(subject), previous)? {
            return Ok(ErasurePreflight::Replay(outcome));
        }
        reject_fenced_subject(&self.store, subject, authenticated_at).await?;
        Ok(ErasurePreflight::Proceed)
    }
    /// Erases once under key and subject locks. Replays never call product deletion again.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::preflight`], [`ErasureError::Database`] when product deletion or the
    /// commit fails, and [`ErasureError::Jobs`] when queueing identity deletion fails. A failed
    /// inline provider call is logged and returns the pending outcome.
    pub async fn erase(
        &self,
        subject: &str,
        authenticated_at: Option<Timestamp>,
        key: &str,
        product: &dyn ProductErasure,
    ) -> Result<ErasureOutcome, ErasureError> {
        validate_request(subject, key)?;
        let key_hashes = self.store.hashes(KEY_DOMAIN, key);
        let key_hash = self.store.hash(KEY_DOMAIN, key);
        let subject_hash = self.store.subject_hash(subject);
        let mut transaction = self.store.pool.begin().await?;
        lock_all(&mut transaction, &key_hashes).await?;
        let previous: Option<(Vec<u8>, Value)> = sqlx::query_as(
            "SELECT subject_hash, response FROM erasure_operations WHERE key_hash = ANY($1)",
        )
        .bind(&key_hashes)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(outcome) = replay_outcome(&self.store.subject_hashes(subject), previous)? {
            return Ok(outcome);
        }
        self.store
            .guard_subject(&mut transaction, subject, authenticated_at)
            .await?;
        product.erase(&mut transaction, subject).await?;
        let outcome = ErasureOutcome {
            status: ErasureState::Pending,
            operation_id: Uuid::now_v7(),
            completed_at: None,
        };
        sqlx::query("INSERT INTO erasure_operations (id, subject_hash, key_hash, state, response) VALUES ($1, $2, $3, 'pending', $4)")
            .bind(outcome.operation_id).bind(&subject_hash).bind(key_hash).bind(serde_json::to_value(&outcome)?)
            .execute(&mut *transaction).await?;
        sqlx::query("INSERT INTO erasure_fences (subject_hash, erased_at) VALUES ($1, now()) ON CONFLICT (subject_hash) DO UPDATE SET erased_at = now()")
            .bind(subject_hash)
            .execute(&mut *transaction)
            .await?;
        let IdentityRetention::Delete {
            deleter,
            provider_id,
            inline_timeout,
            max_attempts,
        } = &self.identity_retention
        else {
            let completed = self
                .store
                .complete(&mut transaction, outcome.operation_id)
                .await?;
            transaction.commit().await?;
            return Ok(completed);
        };
        let payload = serde_json::json!({"subject": subject, "providerId": provider_id, "operationId": outcome.operation_id});
        let job = PostgresJobStore::new(self.store.pool.clone())
            .enqueue_in_transaction(
                &mut transaction,
                NewJob::new(IDENTITY_DELETE_JOB_TYPE, payload, *max_attempts)
                    .idempotency_key(outcome.operation_id.to_string())
                    .run_after(reservation_end(*inline_timeout)?),
            )
            .await?
            .job;
        transaction.commit().await?;
        match self
            .inline(
                subject,
                job.id,
                outcome.clone(),
                deleter.as_ref(),
                *inline_timeout,
            )
            .await
        {
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
        deleter: &dyn IdentityAccountDeleter,
        inline_timeout: Duration,
    ) -> Result<ErasureOutcome, ErasureError> {
        let error_class =
            match tokio::time::timeout(inline_timeout, deleter.delete_account(subject)).await {
                Ok(Ok(())) => None,
                Ok(Err(IdentityDeletionError::Retryable)) => Some("retryable"),
                Ok(Err(IdentityDeletionError::Permanent)) => Some("permanent"),
                Ok(Err(IdentityDeletionError::Unavailable { .. })) => Some("unavailable"),
                Err(_) => Some("timeout"),
            };
        if let Some(error_class) = error_class {
            tracing::warn!(
                operation_id = %outcome.operation_id,
                error_class,
                "inline identity deletion failed",
            );
            self.release_reservation(job_id, outcome.operation_id).await;
            return Ok(outcome);
        }
        match self.complete_reserved(subject, job_id, &outcome).await {
            Ok(completed) => Ok(completed),
            Err(error) => {
                self.release_reservation(job_id, outcome.operation_id).await;
                Err(error)
            }
        }
    }
    /// Completes the operation only while no worker has claimed its job.
    async fn complete_reserved(
        &self,
        subject: &str,
        job_id: Uuid,
        outcome: &ErasureOutcome,
    ) -> Result<ErasureOutcome, ErasureError> {
        let mut transaction = self.store.pool.begin().await?;
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM job_outbox WHERE id = $1 FOR UPDATE")
                .bind(job_id)
                .fetch_optional(&mut *transaction)
                .await?;
        if status.as_deref() != Some("pending") {
            transaction.rollback().await?;
            return Ok(self
                .store
                .status(subject, outcome.operation_id)
                .await?
                .unwrap_or_else(|| outcome.clone()));
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
    /// Makes a reserved job claimable now, so a worker retries without waiting out the reservation.
    ///
    /// A failure only delays the worker until the reservation ends, so it is logged, not returned.
    async fn release_reservation(&self, job_id: Uuid, operation_id: Uuid) {
        let released = sqlx::query(
            "UPDATE job_outbox SET run_after = LEAST(run_after, now()) WHERE id = $1 AND status = 'pending'",
        )
        .bind(job_id)
        .execute(&self.store.pool)
        .await;
        if released.is_err() {
            tracing::warn!(
                %operation_id,
                "inline identity job release failed; a worker claims it after the reservation"
            );
        }
    }
}

/// Keeps workers off a job while its inline attempt runs without a database lock.
fn reservation_end(inline_timeout: Duration) -> Result<Timestamp, ErasureError> {
    let reserved = inline_timeout
        .checked_add(INLINE_RESERVATION_MARGIN)
        .and_then(|reserved| SignedDuration::try_from(reserved).ok())
        .ok_or(ErasureError::Configuration)?;
    Timestamp::now()
        .checked_add(reserved)
        .map_err(|_| ErasureError::Configuration)
}

fn validate_request(subject: &str, key: &str) -> Result<(), ErasureError> {
    let value = axum::http::HeaderValue::from_str(key).map_err(|_| ErasureError::InvalidKey)?;
    IdempotencyKeyRule::new(16, 128)
        .parse(&value)
        .map_err(|_| ErasureError::InvalidKey)?;
    validate_subject(subject)
}

fn validate_subject(subject: &str) -> Result<(), ErasureError> {
    if subject.is_empty() {
        return Err(ErasureError::Configuration);
    }
    if !crate::kernel::is_well_formed_subject(subject) {
        return Err(ErasureError::InvalidSubject);
    }
    Ok(())
}

fn replay_outcome(
    subject_hashes: &[Vec<u8>],
    previous: Option<(Vec<u8>, Value)>,
) -> Result<Option<ErasureOutcome>, ErasureError> {
    let same_subject = previous
        .as_ref()
        .is_some_and(|(owner, _)| subject_hashes.contains(owner));
    match (replay_decision(previous.is_some(), same_subject), previous) {
        (ReplayDecision::Replay, Some((_, response))) => {
            Ok(Some(serde_json::from_value(response)?))
        }
        (ReplayDecision::Conflict, _) => Err(ErasureError::Conflict),
        _ => Ok(None),
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
    fn shared_wire_receipts_require_operation_ids() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../fixtures/erasure/receipts-v1.json"
        ))
        .expect("shared receipts");
        for vector in vectors["cases"].as_array().expect("receipt cases") {
            let mut receipt = vector["receipt"].clone();
            receipt
                .as_object_mut()
                .expect("receipt object")
                .remove("operationId");
            assert!(serde_json::from_value::<ErasureOutcome>(receipt.clone()).is_err());
            receipt["operationId"] = Value::Null;
            assert!(serde_json::from_value::<ErasureOutcome>(receipt).is_err());
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
        outcome.completed_at = Some(Timestamp::now());
        assert_eq!(outcome.status_code(), StatusCode::OK);
        assert!(serde_json::to_value(outcome).expect("serialize")["completedAt"].is_string());
    }
}
