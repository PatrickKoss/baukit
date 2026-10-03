use crate::FakeIdentityAccountDeleter;
use baukit_erasure::{ErasureFuture, IdentityDeletionError};
use serde_json::Value;
use std::{error::Error, sync::Arc};
use uuid::Uuid;

/// Counts and fence state from the product's actual persistence adapter.
#[derive(Clone, Debug)]
pub struct IdentityErasureSnapshot {
    /// Rows in the product's complete owned-resource inventory.
    pub product_rows: u64,
    /// Identity deletion jobs containing this subject.
    pub identity_jobs: u64,
    /// All rows containing the raw subject, including outbox payloads.
    pub raw_subject_rows: u64,
    /// Whether subject resolution is fenced.
    pub fenced: bool,
}
/// HTTP observation from the real endpoint wiring.
#[derive(Clone, Debug, PartialEq)]
pub struct IdentityErasureResponse {
    /// HTTP status code.
    pub status: u16,
    /// Decoded product response body.
    pub body: Value,
}
/// Product endpoint and worker adapter for identity erasure conformance.
pub trait IdentityErasureAdapter {
    /// Failure type, excluded from safe conformance diagnostics.
    type Error: Error + Send + Sync + 'static;
    /// Seeds the owned-resource graph and installs the supplied provider fake.
    fn seed<'a>(
        &'a mut self,
        subject: &'a str,
        fake: Arc<FakeIdentityAccountDeleter>,
    ) -> ErasureFuture<'a, Result<(), Self::Error>>;
    /// Calls DELETE through the product's real authenticated router.
    fn delete<'a>(
        &'a self,
        subject: &'a str,
        key: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>>;
    /// Calls the subject-authorized status endpoint through that router.
    fn status<'a>(
        &'a self,
        subject: &'a str,
        operation: Uuid,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>>;
    /// Executes the registered worker until the identity operation completes.
    fn run_worker(&self) -> ErasureFuture<'_, Result<(), Self::Error>>;
    /// Calls a profile-resolving authenticated endpoint, such as GET /me.
    fn resolve<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureResponse, Self::Error>>;
    /// Counts the product inventory, identity jobs, and every raw-subject location.
    fn snapshot<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<IdentityErasureSnapshot, Self::Error>>;
}
/// Safe conformance failure, without subjects or provider diagnostics.
#[derive(Debug, thiserror::Error)]
#[error("identity erasure conformance failed: {0}")]
pub struct IdentityErasureConformanceError(String);
fn require(condition: bool, message: &str) -> Result<(), IdentityErasureConformanceError> {
    if condition {
        Ok(())
    } else {
        Err(IdentityErasureConformanceError(message.into()))
    }
}
fn failed<E>(_: E) -> IdentityErasureConformanceError {
    IdentityErasureConformanceError("adapter call failed".into())
}

/// Verifies durable acceptance, replay, conflict, fences, worker completion and subject removal.
pub async fn check_identity_erasure_conformance<A: IdentityErasureAdapter>(
    adapter: &mut A,
    subject: &str,
    foreign_subject: &str,
) -> Result<(), IdentityErasureConformanceError> {
    require(
        !subject.is_empty() && !foreign_subject.is_empty() && subject != foreign_subject,
        "two distinct subjects are required",
    )?;
    let fake = Arc::new(FakeIdentityAccountDeleter::default());
    fake.push_outcome(Err(IdentityDeletionError::Retryable));
    adapter.seed(subject, fake.clone()).await.map_err(failed)?;
    let seeded = adapter.snapshot(subject).await.map_err(failed)?;
    require(
        seeded.product_rows > 0 && !seeded.fenced,
        "fixture must seed unfenced product rows",
    )?;
    let key = Uuid::now_v7().to_string();
    let pending = adapter.delete(subject, &key).await.map_err(failed)?;
    require(
        pending.status == 202 && pending.body["status"] == "pending",
        "IdP failure must return durable pending acceptance",
    )?;
    let operation = pending.body["operationId"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .ok_or_else(|| failed(()))?;
    let snapshot = adapter.snapshot(subject).await.map_err(failed)?;
    require(
        snapshot.product_rows == 0 && snapshot.identity_jobs == 1 && snapshot.fenced,
        "pending erasure must remove product rows, fence the subject and retain one job",
    )?;
    require(
        fake.calls() == [subject],
        "inline provider deletion must be attempted once",
    )?;
    require(
        adapter
            .status(subject, operation)
            .await
            .map_err(failed)?
            .body
            == pending.body,
        "status must report the pending receipt",
    )?;
    require(
        adapter.delete(subject, &key).await.map_err(failed)? == pending,
        "pending replay must return the same response",
    )?;
    let conflict = adapter
        .delete(foreign_subject, &key)
        .await
        .map_err(failed)?;
    require(
        conflict.status == 409 && conflict.body["error"]["code"] == "erasure_idempotency_conflict",
        "cross-identity key reuse must conflict",
    )?;
    require(
        adapter
            .status(foreign_subject, operation)
            .await
            .map_err(failed)?
            .status
            == 404,
        "foreign receipt lookup must return 404",
    )?;
    let fenced = adapter.resolve(subject).await.map_err(failed)?;
    require(
        fenced.status == 401 && fenced.body["error"]["code"] == "profile_erased",
        "fenced subject resolution must reject profile recreation",
    )?;
    adapter.run_worker().await.map_err(failed)?;
    let completed = adapter.status(subject, operation).await.map_err(failed)?;
    require(
        completed.status == 200
            && completed.body["status"] == "completed"
            && completed.body["completedAt"].is_string(),
        "worker must complete the operation",
    )?;
    require(
        fake.calls() == [subject, subject],
        "worker must call the registered identity deleter",
    )?;
    let replay = adapter.delete(subject, &key).await.map_err(failed)?;
    require(
        replay.status == 200 && replay.body == completed.body,
        "completed replay must return the stored receipt",
    )?;
    require(
        adapter.delete(subject, &key).await.map_err(failed)? == replay,
        "completion timestamp must remain stable",
    )?;
    let snapshot = adapter.snapshot(subject).await.map_err(failed)?;
    require(
        snapshot.product_rows == 0
            && snapshot.identity_jobs == 0
            && snapshot.raw_subject_rows == 0
            && snapshot.fenced,
        "completion must remove all raw subjects and preserve the fence",
    )?;
    Ok(())
}
