use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use baukit_erasure::ErasureState;
use baukit_jobs::{
    ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler, NewJob, PostgresJobStore,
};
use chrono::{DateTime, SubsecRound, TimeDelta, Utc};
use serde::Deserialize;
use sqlx::{Acquire, PgConnection, Postgres, Transaction};
use uuid::Uuid;

use super::{
    DataSubjectError, DataSubjectProduct, ERASURE_JOB_TYPE, EXPORT_JOB_TYPE, FINALIZATION_JOB_TYPE,
    LocalSubject, PostgresDataSubjectStore, store::load_for_execution,
};
use crate::{
    CONTRACT_VERSION, OperationKind, OperationResource, OperationStatus, RECEIPT_TYPE,
    ReceiptPayload, codes,
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExecutionPayload {
    operation_id: Uuid,
    subjects: Vec<LocalSubject>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FinalizationPayload {
    operation_id: Uuid,
    #[serde(default)]
    check_count: u32,
}

/// Lease-aware erasure, export and identity-completion worker.
/// Register with the product's `baukit_jobs::WorkerRunner`.
#[derive(Clone)]
pub struct DataSubjectHandler {
    store: PostgresDataSubjectStore,
    product: Arc<dyn DataSubjectProduct>,
}

impl DataSubjectHandler {
    /// Uses the same store and product adapter as the authenticated router.
    pub fn new(store: PostgresDataSubjectStore, product: Arc<dyn DataSubjectProduct>) -> Self {
        Self { store, product }
    }

    async fn execute(
        &self,
        job: &ClaimedJob,
        cancellation: &JobCancellation,
        id: Uuid,
        subjects: Option<Vec<LocalSubject>>,
        check_count: u32,
    ) -> Result<(), DataSubjectError> {
        let jobs = PostgresJobStore::new(self.store.pool.clone());
        let mut transaction = self.store.pool.begin().await?;
        if !jobs
            .check_lease_in_transaction(
                &mut transaction,
                job.id,
                cancellation.worker_id(),
                Utc::now(),
            )
            .await?
        {
            return Err(DataSubjectError::Jobs);
        }
        let mut identity_deadline = None;
        if let Some(mut operation) = load_for_execution(&mut transaction, id).await?
            && operation.status == OperationStatus::Pending
        {
            if operation.kind == OperationKind::Export {
                if job.job_type != EXPORT_JOB_TYPE {
                    return Err(DataSubjectError::Result);
                }
                self.export_result(
                    &mut transaction,
                    operation,
                    subjects.ok_or(DataSubjectError::Result)?,
                )
                .await?;
            } else if let Some(subjects) = subjects {
                let mut fenced = HashSet::new();
                let mut waiting = false;
                for subject in &subjects {
                    if let Some(authentication_subject) = &subject.authentication_subject
                        && fenced.insert(authentication_subject)
                    {
                        let identity = self
                            .store
                            .config
                            .erasure
                            .fence_as_operator_in_transaction(
                                &mut transaction,
                                authentication_subject,
                            )
                            .await
                            .map_err(|_| DataSubjectError::Database)?;
                        if identity.status == ErasureState::Pending {
                            waiting = true;
                            sqlx::query("INSERT INTO data_subject_identity_operations (operation_id, identity_operation_id) VALUES ($1, $2)")
                                .bind(id).bind(identity.operation_id).execute(&mut *transaction).await?;
                        }
                    }
                    let report = self.product.erase(&mut transaction, subject).await?;
                    operation.categories.extend(report.categories);
                    operation.restrictions.extend(report.restrictions);
                }
                if waiting {
                    let started_at: DateTime<Utc> = sqlx::query_scalar("UPDATE data_subject_operations SET categories = $2, restrictions = $3, identity_deletion_started_at = clock_timestamp() WHERE id = $1 RETURNING identity_deletion_started_at")
                        .bind(id).bind(serde_json::to_value(operation.categories)?)
                        .bind(serde_json::to_value(operation.restrictions)?)
                        .fetch_one(&mut *transaction).await?;
                    identity_deadline =
                        Some(started_at + self.store.config.identity_deletion_timeout);
                } else {
                    operation.status = result_status(subjects.is_empty(), &operation);
                    self.sign_result(&mut transaction, operation).await?;
                }
            } else {
                let started_at: Option<DateTime<Utc>> = sqlx::query_scalar("SELECT identity_deletion_started_at FROM data_subject_operations WHERE id = $1")
                    .bind(id).fetch_one(&mut *transaction).await?;
                let deadline = started_at.ok_or(DataSubjectError::Result)?
                    + self.store.config.identity_deletion_timeout;
                let states: Vec<String> = sqlx::query_scalar("SELECT identity_operation.state FROM erasure_operations AS identity_operation JOIN data_subject_identity_operations AS dependency ON dependency.identity_operation_id = identity_operation.id WHERE dependency.operation_id = $1")
                    .bind(id).fetch_all(&mut *transaction).await?;
                if states.is_empty() {
                    return Err(DataSubjectError::Result);
                }
                if states.iter().any(|state| state == "failed") || Utc::now() >= deadline {
                    sqlx::query("UPDATE data_subject_operations SET status = 'failed', completed_at = now(), error_code = $2 WHERE id = $1")
                        .bind(id).bind(codes::OPERATION_FAILED).execute(&mut *transaction).await?;
                } else if states.iter().any(|state| state != "completed") {
                    identity_deadline = Some(deadline);
                } else {
                    operation.status = result_status(false, &operation);
                    self.sign_result(&mut transaction, operation).await?;
                }
            }
        }
        if !jobs
            .complete_in_transaction(
                &mut transaction,
                job.id,
                cancellation.worker_id(),
                Utc::now(),
            )
            .await?
        {
            return Err(DataSubjectError::Jobs);
        }
        sqlx::query("DELETE FROM job_outbox WHERE id = $1")
            .bind(job.id)
            .execute(&mut *transaction)
            .await?;
        if let Some(deadline) = identity_deadline {
            let delay = TimeDelta::seconds((1_i64 << check_count.min(6)).min(60));
            let run_after = (Utc::now() + delay).min(deadline);
            jobs.enqueue_in_transaction(
                &mut transaction,
                NewJob::new(
                    FINALIZATION_JOB_TYPE,
                    serde_json::json!({"operationId": id, "checkCount": check_count}),
                    self.store.config.max_attempts,
                )
                .idempotency_key(id.to_string())
                .run_after(run_after),
            )
            .await?;
        }
        transaction.commit().await?;
        cancellation.mark_completed_in_transaction();
        Ok(())
    }

    async fn export_result(
        &self,
        transaction: &mut Transaction<'_, Postgres>,
        mut operation: OperationResource,
        subjects: Vec<LocalSubject>,
    ) -> Result<(), DataSubjectError> {
        let product = self
            .store
            .config
            .export_product
            .as_ref()
            .ok_or(DataSubjectError::Configuration)?;
        let mut execution = transaction.begin().await?;
        let mut categories = BTreeMap::new();
        for subject in &subjects {
            for category in product.export(&mut execution, subject).await? {
                categories
                    .entry(category.name)
                    .or_insert_with(Vec::new)
                    .extend(category.records);
            }
        }
        let split =
            super::export::split(categories, self.store.config.limits.max_export_part_bytes);
        let (categories, parts) = match split {
            Ok(result) => result,
            Err(DataSubjectError::ExportRecordTooLarge) => {
                execution.rollback().await?;
                sqlx::query("UPDATE data_subject_operations SET status = 'failed', completed_at = now(), error_code = $2 WHERE id = $1")
                    .bind(operation.operation_id).bind(codes::EXPORT_RECORD_TOO_LARGE)
                    .execute(&mut **transaction).await?;
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        operation.categories = categories;
        operation.status = result_status(subjects.is_empty(), &operation);
        operation.parts = parts.iter().map(|part| part.metadata.clone()).collect();
        let completed_at = Utc::now().trunc_subsecs(6);
        operation.completed_at = Some(completed_at);
        let expires_at = completed_at
            .checked_add_signed(self.store.config.export_retention)
            .ok_or(DataSubjectError::Configuration)?;
        for part in parts {
            sqlx::query("INSERT INTO data_subject_export_parts (operation_id, part_number, json_bytes, sha256, bytes, expires_at) VALUES ($1, $2, $3, $4, $5, $6)")
                .bind(operation.operation_id).bind(i64::from(part.metadata.part_number))
                .bind(part.bytes).bind(part.metadata.sha256).bind(part.metadata.bytes as i64).bind(expires_at)
                .execute(&mut *execution).await?;
        }
        self.sign_result(&mut execution, operation).await?;
        execution.commit().await?;
        Ok(())
    }

    async fn sign_result(
        &self,
        connection: &mut PgConnection,
        operation: OperationResource,
    ) -> Result<(), DataSubjectError> {
        let completed_at = operation
            .completed_at
            .unwrap_or_else(|| Utc::now().trunc_subsecs(6));
        let receipt = self
            .store
            .config
            .signer
            .sign(&ReceiptPayload {
                receipt_type: RECEIPT_TYPE.into(),
                contract_version: CONTRACT_VERSION.into(),
                operation_id: operation.operation_id,
                kind: operation.kind,
                status: operation.status,
                subject_reference: operation.subject_reference,
                service: self.store.config.service.clone(),
                created_at: operation.created_at,
                completed_at,
                categories: operation.categories.clone(),
                restrictions: operation.restrictions.clone(),
                parts: operation.parts.clone(),
            })
            .map_err(|_| DataSubjectError::Result)?;
        sqlx::query("UPDATE data_subject_operations SET status = $2, completed_at = $3, categories = $4, restrictions = $5, receipt = $6, parts = $7, export_expires_at = $8 WHERE id = $1")
            .bind(operation.operation_id).bind(serde_json::to_value(operation.status)?.as_str().ok_or(DataSubjectError::Result)?)
            .bind(completed_at).bind(serde_json::to_value(operation.categories)?)
            .bind(serde_json::to_value(operation.restrictions)?).bind(serde_json::to_value(receipt)?)
            .bind(serde_json::to_value(operation.parts)?)
            .bind(if operation.kind == OperationKind::Export && operation.status == OperationStatus::Completed {
                Some(completed_at.checked_add_signed(self.store.config.export_retention).ok_or(DataSubjectError::Configuration)?)
            } else { None })
            .execute(connection).await?;
        Ok(())
    }
}

fn result_status(not_found: bool, operation: &OperationResource) -> OperationStatus {
    if not_found {
        OperationStatus::NotFound
    } else if operation.restrictions.is_empty() {
        OperationStatus::Completed
    } else {
        OperationStatus::Restricted
    }
}

impl JobHandler for DataSubjectHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[ERASURE_JOB_TYPE, EXPORT_JOB_TYPE, FINALIZATION_JOB_TYPE]
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            let (id, subjects, check_count) = match job.job_type.as_str() {
                ERASURE_JOB_TYPE | EXPORT_JOB_TYPE => {
                    let payload: ExecutionPayload = serde_json::from_value(job.payload.clone())
                        .map_err(|_| {
                            JobError::permanent("invalid data subject execution payload")
                        })?;
                    (payload.operation_id, Some(payload.subjects), 0)
                }
                FINALIZATION_JOB_TYPE => {
                    let payload: FinalizationPayload = serde_json::from_value(job.payload.clone())
                        .map_err(|_| {
                            JobError::permanent("invalid data subject finalization payload")
                        })?;
                    (
                        payload.operation_id,
                        None,
                        payload.check_count.saturating_add(1),
                    )
                }
                _ => return Err(JobError::permanent("invalid data subject job kind")),
            };
            self.execute(job, &cancellation, id, subjects, check_count)
                .await
                .map_err(|_| JobError::retryable("data subject execution unavailable"))
        })
    }
}
