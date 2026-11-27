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
impl IdentityDeletionHandler {
    /// Fails the operation of a payload the worker will never run, so nothing waits on it.
    /// A store failure is retryable, so a later attempt records the failure again.
    async fn reject_payload(&self, job: &ClaimedJob, error: JobError) -> JobError {
        let Some(operation_id) = recoverable_operation_id(&job.payload) else {
            tracing::error!(job_id = %job.id, "identity deletion payload has no operation id");
            return error;
        };
        match self.store.fail(operation_id).await {
            Ok(_) => {
                tracing::error!(
                    job_id = %job.id,
                    operation_id = %operation_id,
                    "identity deletion payload rejected, operation failed",
                );
                error
            }
            Err(_) => JobError::retryable("erasure failure receipt unavailable"),
        }
    }
}

fn recoverable_operation_id(value: &serde_json::Value) -> Option<Uuid> {
    value.get("operationId")?.as_str()?.parse().ok()
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
            let payload = match parse_payload(&job.payload, &self.provider_id) {
                Ok(payload) => payload,
                Err(error) => return Err(self.reject_payload(job, error).await),
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_id_survives_an_invalid_subject_but_not_a_malformed_id() {
        let id = Uuid::now_v7();
        let payload = serde_json::json!({"subject": "a\nb", "providerId": "p", "operationId": id});
        assert!(parse_payload(&payload, "p").is_err());
        assert_eq!(recoverable_operation_id(&payload), Some(id));
        for unreadable in [
            serde_json::json!({"subject": "a"}),
            serde_json::json!({"operationId": "not-a-uuid"}),
            serde_json::json!("garbage"),
        ] {
            assert_eq!(recoverable_operation_id(&unreadable), None);
        }
    }
}
