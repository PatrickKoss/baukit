use crate::{
    IDENTITY_DELETE_JOB_TYPE, IdentityAccountDeleter, IdentityDeletionError, PostgresErasureStore,
};
use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler, PostgresJobStore};
use jiff::Timestamp;

use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

/// A validated identity deletion job payload.
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Payload {
    /// Provider subject to delete.
    pub subject: String,
    /// Provider the producer selected.
    pub provider_id: String,
    /// Erasure operation to complete.
    pub operation_id: Uuid,
}

/// Decodes a job payload for `provider_id`. Only a well-formed subject is returned.
///
/// # Errors
///
/// A permanent [`JobError`] for a malformed payload, another provider, or a subject that is
/// not exactly one well-formed provider path segment.
pub fn parse_payload(value: &serde_json::Value, provider_id: &str) -> Result<Payload, JobError> {
    let payload: Payload = serde_json::from_value(value.clone())
        .map_err(|_| JobError::permanent("invalid identity deletion payload"))?;
    if payload.provider_id != provider_id {
        return Err(JobError::permanent("identity deletion provider mismatch"));
    }
    if !crate::kernel::is_well_formed_subject(&payload.subject) {
        return Err(JobError::permanent("invalid identity deletion payload"));
    }
    Ok(payload)
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
            if job.job_type != IDENTITY_DELETE_JOB_TYPE {
                return Err(JobError::permanent("identity deletion provider mismatch"));
            }
            let payload = parse_payload(&job.payload, &self.provider_id)?;
            match self.deleter.delete_account(&payload.subject).await {
                Ok(()) => {}
                Err(IdentityDeletionError::Retryable) => {
                    return Err(JobError::retryable(
                        "identity deletion temporarily unavailable",
                    ));
                }
                Err(IdentityDeletionError::Unavailable { retry_after }) => {
                    return Err(JobError::deferred(
                        "identity provider unavailable",
                        retry_after,
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
                .complete_lease_in_transaction(
                    &mut transaction,
                    cancellation.lease(),
                    Timestamp::now(),
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
