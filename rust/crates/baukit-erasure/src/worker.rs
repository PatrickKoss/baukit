use crate::{
    IDENTITY_DELETE_JOB_TYPE, IdentityAccountDeleter, IdentityDeletionError, PostgresErasureStore,
};
use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler, PostgresJobStore};
use chrono::Utc;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Payload {
    subject: String,
    provider_id: String,
    operation_id: Uuid,
}

/// Lease-aware identity deletion handler. WorkerRunner supplies retry backoff.
#[derive(Clone)]
pub struct IdentityDeletionHandler {
    store: PostgresErasureStore,
    provider_id: String,
    deleter: Arc<dyn IdentityAccountDeleter>,
}
impl IdentityDeletionHandler {
    /// Registers the provider used by the producer.
    pub fn new(
        store: PostgresErasureStore,
        provider_id: String,
        deleter: Arc<dyn IdentityAccountDeleter>,
    ) -> Self {
        Self {
            store,
            provider_id,
            deleter,
        }
    }
}
impl JobHandler for IdentityDeletionHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[IDENTITY_DELETE_JOB_TYPE]
    }
    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            let payload: Payload = serde_json::from_value(job.payload.clone())
                .map_err(|_| JobError::permanent("invalid identity deletion payload"))?;
            if job.job_type != IDENTITY_DELETE_JOB_TYPE || payload.provider_id != self.provider_id {
                return Err(JobError::permanent("identity deletion provider mismatch"));
            }
            match self.deleter.delete_account(&payload.subject).await {
                Ok(()) => {}
                Err(IdentityDeletionError::Retryable) => {
                    return Err(JobError::retryable(
                        "identity deletion temporarily unavailable",
                    ));
                }
                Err(IdentityDeletionError::Permanent) => {
                    return Err(JobError::permanent(
                        "identity deletion rejected permanently",
                    ));
                }
            }
            let mut transaction = self
                .store
                .pool()
                .begin()
                .await
                .map_err(|_| JobError::retryable("erasure completion database unavailable"))?;
            let owned = PostgresJobStore::new(self.store.pool().clone())
                .complete_in_transaction(
                    &mut transaction,
                    job.id,
                    cancellation.worker_id(),
                    Utc::now(),
                )
                .await
                .map_err(|_| JobError::retryable("erasure completion job store unavailable"))?;
            if !owned {
                return Err(JobError::retryable("erasure completion lease lost"));
            }
            self.store
                .complete(&mut transaction, payload.operation_id)
                .await
                .map_err(|_| JobError::retryable("erasure completion receipt unavailable"))?;
            sqlx::query("DELETE FROM job_outbox WHERE id = $1")
                .bind(job.id)
                .execute(&mut *transaction)
                .await
                .map_err(|_| JobError::retryable("erasure completed job cleanup unavailable"))?;
            transaction
                .commit()
                .await
                .map_err(|_| JobError::retryable("erasure completion commit unavailable"))?;
            cancellation.mark_completed_in_transaction();
            Ok(())
        })
    }
}
