use std::sync::Arc;

use aws_lc_rs::hmac;
use baukit_jobs::{NewJob, PostgresJobStore};
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    DataSubjectConfig, DataSubjectError, DataSubjectProduct, ERASURE_JOB_TYPE,
    FINALIZATION_JOB_TYPE, MAX_PURGE_BATCH_SIZE, config::SharedConfig,
};
use crate::{
    DiscoveryDocument, NormalizedRequest, OperationError, OperationKind, OperationResource,
};

/// PostgreSQL operation storage. Products apply the published migration first.
#[derive(Clone)]
pub struct PostgresDataSubjectStore {
    pub(super) pool: PgPool,
    pub(super) config: SharedConfig,
}

impl PostgresDataSubjectStore {
    /// Validates all configuration before accepting work.
    /// The erasure service must use the same PostgreSQL database as this pool.
    pub fn new(pool: PgPool, config: DataSubjectConfig) -> Result<Self, DataSubjectError> {
        config.validate()?;
        Ok(Self {
            pool,
            config: Arc::new(config),
        })
    }

    /// Product pool used for operation and job transactions.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Public configuration, including exactly the operations this adapter implements.
    pub fn discovery(&self) -> DiscoveryDocument {
        self.config.discovery()
    }

    fn hash(&self, domain: &[u8], inputs: &[&[u8]]) -> Vec<u8> {
        let key = hmac::Key::new(hmac::HMAC_SHA256, self.config.hash_key.expose().as_bytes());
        let mut context = hmac::Context::with_key(&key);
        context.update(domain);
        for input in inputs {
            context.update(&(input.len() as u64).to_be_bytes());
            context.update(input);
        }
        context.sign().as_ref().to_vec()
    }

    /// Resolves and queues one erasure atomically. Inputs remain transient.
    /// Callers must validate the idempotency key and authorize the client first.
    pub async fn start_erasure(
        &self,
        client_id: &str,
        key: &str,
        request: &NormalizedRequest,
        product: &dyn DataSubjectProduct,
    ) -> Result<OperationResource, DataSubjectError> {
        let key_hash = self.hash(
            b"data-subject-key\0",
            &[client_id.as_bytes(), b"erasure", key.as_bytes()],
        );
        let normalized = Zeroizing::new(serde_json::to_vec(request)?);
        let request_hash = self.hash(b"data-subject-request\0", &[&normalized]);
        let mut transaction = self.pool.begin().await?;
        let mut lock_bytes = [0; 8];
        lock_bytes.copy_from_slice(&key_hash[..8]);
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(i64::from_be_bytes(lock_bytes))
            .execute(&mut *transaction)
            .await?;
        let previous = sqlx::query("SELECT id, kind, status, subject_reference, created_at, completed_at, categories, restrictions, parts, receipt, error_code, request_hash FROM data_subject_operations WHERE client_id = $1 AND kind = 'erasure' AND key_hash = $2")
            .bind(client_id).bind(&key_hash).fetch_optional(&mut *transaction).await?;
        if let Some(row) = previous {
            if row.try_get::<Vec<u8>, _>("request_hash")? != request_hash {
                return Err(DataSubjectError::Conflict);
            }
            return resource(row);
        }
        let mut subjects = product
            .resolve(&mut transaction, request.identifiers())
            .await?;
        subjects.sort_by(|left, right| {
            (&left.authentication_subject, &left.subject_id)
                .cmp(&(&right.authentication_subject, &right.subject_id))
        });
        subjects.dedup();
        let id = Uuid::now_v7();
        let row = sqlx::query("INSERT INTO data_subject_operations (id, kind, client_id, key_hash, request_hash, subject_reference, status, purge_after) VALUES ($1, 'erasure', $2, $3, $4, $5, 'pending', $6) RETURNING id, kind, status, subject_reference, created_at, completed_at, categories, restrictions, parts, receipt, error_code")
            .bind(id).bind(client_id).bind(key_hash).bind(request_hash).bind(request.subject_reference())
            .bind(Utc::now().checked_add_signed(self.config.operation_retention).ok_or(DataSubjectError::Configuration)?)
            .fetch_one(&mut *transaction).await?;
        PostgresJobStore::new(self.pool.clone())
            .enqueue_in_transaction(
                &mut transaction,
                NewJob::new(
                    ERASURE_JOB_TYPE,
                    serde_json::json!({"operationId": id, "subjects": subjects}),
                    self.config.max_attempts,
                )
                .idempotency_key(id.to_string()),
            )
            .await?;
        let operation = resource(row)?;
        transaction.commit().await?;
        Ok(operation)
    }

    /// Client- and kind-isolated operation lookup. Foreign operations are absent.
    pub async fn status(
        &self,
        client_id: &str,
        kind: OperationKind,
        id: Uuid,
    ) -> Result<Option<OperationResource>, DataSubjectError> {
        let kind = serde_json::to_value(kind)?;
        sqlx::query("SELECT id, kind, status, subject_reference, created_at, completed_at, categories, restrictions, parts, receipt, error_code FROM data_subject_operations WHERE id = $1 AND client_id = $2 AND kind = $3")
            .bind(id).bind(client_id).bind(kind.as_str().ok_or(DataSubjectError::Result)?)
            .fetch_optional(&self.pool).await?.map(resource).transpose()
    }

    /// Deletes at most `batch_size` expired operations and their execution payloads.
    /// Schedule repeated calls using a product job and `baukit_jobs::FixedUtcInterval`.
    /// Zero or a size above [`MAX_PURGE_BATCH_SIZE`] is a configuration error.
    pub async fn purge_expired(
        &self,
        now: DateTime<Utc>,
        batch_size: u32,
    ) -> Result<u64, DataSubjectError> {
        if batch_size == 0 || batch_size > MAX_PURGE_BATCH_SIZE {
            return Err(DataSubjectError::Configuration);
        }
        let mut transaction = self.pool.begin().await?;
        // Execution takes the job lock before the operation lock too.
        let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM data_subject_operations WHERE purge_after < $1 ORDER BY purge_after, id LIMIT $2")
            .bind(now).bind(i64::from(batch_size)).fetch_all(&mut *transaction).await?;
        let locked_jobs: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM job_outbox WHERE job_type = ANY($1) AND idempotency_key = ANY($2) ORDER BY id FOR UPDATE")
            .bind(vec![ERASURE_JOB_TYPE, FINALIZATION_JOB_TYPE]).bind(ids.iter().map(ToString::to_string).collect::<Vec<_>>())
            .fetch_all(&mut *transaction).await?;
        sqlx::query("DELETE FROM job_outbox WHERE id = ANY($1)")
            .bind(locked_jobs)
            .execute(&mut *transaction)
            .await?;
        let count = sqlx::query(
            "DELETE FROM data_subject_operations WHERE id = ANY($1) AND purge_after < $2",
        )
        .bind(ids)
        .bind(now)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        transaction.commit().await?;
        Ok(count)
    }
}

pub(super) async fn load_for_execution(
    connection: &mut PgConnection,
    id: Uuid,
) -> Result<Option<OperationResource>, DataSubjectError> {
    sqlx::query("SELECT id, kind, status, subject_reference, created_at, completed_at, categories, restrictions, parts, receipt, error_code FROM data_subject_operations WHERE id = $1 AND kind = 'erasure' FOR UPDATE")
        .bind(id).fetch_optional(connection).await?.map(resource).transpose()
}

fn resource(row: PgRow) -> Result<OperationResource, DataSubjectError> {
    let receipt: Option<Value> = row.try_get("receipt")?;
    let error_code: Option<String> = row.try_get("error_code")?;
    Ok(OperationResource {
        operation_id: row.try_get("id")?,
        kind: serde_json::from_value(Value::String(row.try_get("kind")?))?,
        status: serde_json::from_value(Value::String(row.try_get("status")?))?,
        subject_reference: row.try_get("subject_reference")?,
        created_at: row.try_get("created_at")?,
        completed_at: row.try_get("completed_at")?,
        categories: serde_json::from_value(row.try_get("categories")?)?,
        restrictions: serde_json::from_value(row.try_get("restrictions")?)?,
        parts: serde_json::from_value(row.try_get("parts")?)?,
        receipt: receipt.map(serde_json::from_value).transpose()?,
        error: error_code.map(|code| OperationError { code }),
    })
}
