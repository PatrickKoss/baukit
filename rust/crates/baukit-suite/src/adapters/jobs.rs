use crate::{
    domain::*,
    ports::*,
    services::{SuiteDeliveryRunner, SuiteServiceError},
};
use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler};
use chrono::Utc;
use std::sync::Arc;
pub struct SuiteJobHandler {
    service: Arc<dyn SuiteDeliveryRunner>,
}
impl SuiteJobHandler {
    pub fn new(service: Arc<dyn SuiteDeliveryRunner>) -> Self {
        Self { service }
    }
}
impl JobHandler for SuiteJobHandler {
    fn job_types(&self) -> &'static [&'static str] {
        &[SUITE_EVENTS_DELIVER_JOB_TYPE, SUITE_LINKS_REVOKE_JOB_TYPE]
    }
    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            let result = async {
                let action = match job.job_type.as_str() {
                    SUITE_EVENTS_DELIVER_JOB_TYPE => {
                        self.service
                            .deliver(
                                serde_json::from_value(job.payload.clone())
                                    .map_err(|_| JobError::permanent("suite_payload_invalid"))?,
                                Utc::now(),
                                job.id,
                                cancellation.worker_id(),
                            )
                            .await
                    }
                    SUITE_LINKS_REVOKE_JOB_TYPE => {
                        self.service
                            .revoke(
                                serde_json::from_value(job.payload.clone())
                                    .map_err(|_| JobError::permanent("suite_payload_invalid"))?,
                                Utc::now(),
                            )
                            .await
                    }
                    _ => return Err(JobError::permanent("suite_job_type_unsupported")),
                }
                .map_err(service_job_error)?;
                match action {
                    DeliveryAction::Retry { after: Some(delay) } => {
                        Err(JobError::retryable_after("suite_peer_unreachable", delay))
                    }
                    DeliveryAction::Retry { after: None } => {
                        Err(JobError::retryable("suite_peer_unreachable"))
                    }
                    a if a.permanent_code().is_some() => Err(JobError::permanent(
                        a.permanent_code().unwrap_or("suite_rejected"),
                    )),
                    DeliveryAction::Delivered if job.job_type == SUITE_EVENTS_DELIVER_JOB_TYPE => {
                        cancellation.mark_completed_in_transaction();
                        Ok(())
                    }
                    _ => Ok(()),
                }
            }
            .await;
            if let Err(error) = &result {
                tracing::warn!(job_id=%job.id, code=%error, "Suite job failed");
            }
            result
        })
    }
}

fn service_job_error(error: SuiteServiceError) -> JobError {
    tracing::warn!(code = error.code(), "Suite job service failed");
    match error {
        SuiteServiceError::Protocol(
            LinkProtocolError::Disabled | LinkProtocolError::PeerUnknown,
        ) => JobError::permanent("suite_link_disabled"),
        SuiteServiceError::Cipher | SuiteServiceError::Store(SuiteStoreError::InvalidData(_)) => {
            JobError::permanent(error.code())
        }
        _ => JobError::retryable(error.code()),
    }
}

/// Runs cleanup on UTC hour boundaries until the shutdown channel closes or becomes true.
/// Return errors to the product's worker supervisor.
pub async fn run_hourly_cleanup(
    service: Arc<dyn SuiteDeliveryRunner>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), SuiteServiceError> {
    let interval = baukit_jobs::FixedUtcInterval::new(std::time::Duration::from_secs(60 * 60))
        .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
    let mut slot = interval
        .slot_at(Utc::now())
        .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        slot = interval
            .next_slot(slot, Utc::now())
            .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
        let wait = (slot.starts_at() - Utc::now()).to_std().unwrap_or_default();
        tokio::select! {
            () = shutdown_requested(&mut shutdown) => { return Ok(()); }
            () = tokio::time::sleep(wait) => { service.cleanup(Utc::now()).await?; }
        }
    }
}

async fn shutdown_requested(shutdown: &mut tokio::sync::watch::Receiver<bool>) {
    while !*shutdown.borrow_and_update() {
        if shutdown.changed().await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct CleanupRunner(std::sync::atomic::AtomicUsize);
    #[async_trait::async_trait]
    impl SuiteDeliveryRunner for CleanupRunner {
        async fn deliver(
            &self,
            _: SuiteDeliverJobV1,
            _: chrono::DateTime<Utc>,
            _: uuid::Uuid,
            _: &str,
        ) -> Result<DeliveryAction, SuiteServiceError> {
            Err(SuiteStoreError::InvalidData("unexpected delivery during cleanup".into()).into())
        }
        async fn revoke(
            &self,
            _: SuiteRevokeJobV1,
            _: chrono::DateTime<Utc>,
        ) -> Result<DeliveryAction, SuiteServiceError> {
            Err(SuiteStoreError::InvalidData("unexpected revoke during cleanup".into()).into())
        }
        async fn cleanup(
            &self,
            _: chrono::DateTime<Utc>,
        ) -> Result<SuiteCleanupOutcome, SuiteServiceError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err(SuiteStoreError::Storage("cleanup database unavailable".into()).into())
        }
    }
    #[tokio::test(start_paused = true)]
    async fn cleanup_waits_for_the_hour_and_propagates_failure_after_false_notifications() {
        let service = Arc::new(CleanupRunner(std::sync::atomic::AtomicUsize::new(0)));
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run_hourly_cleanup(service.clone(), receiver));
        tokio::task::yield_now().await;
        sender.send(false).expect("notification");
        tokio::task::yield_now().await;
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        tokio::time::advance(std::time::Duration::from_secs(3600)).await;
        let error = task.await.expect("task").expect_err("cleanup failure");
        assert!(matches!(
            error,
            SuiteServiceError::Store(SuiteStoreError::Storage(_))
        ));
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 1);
    }
    #[tokio::test(start_paused = true)]
    async fn cleanup_stops_on_shutdown_or_channel_close_without_accessing_storage() {
        let service = Arc::new(CleanupRunner(std::sync::atomic::AtomicUsize::new(0)));
        let (sender, receiver) = tokio::sync::watch::channel(true);
        run_hourly_cleanup(service.clone(), receiver)
            .await
            .expect("already shut down");
        drop(sender);
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run_hourly_cleanup(service.clone(), receiver));
        tokio::task::yield_now().await;
        drop(sender);
        task.await.expect("task").expect("closed channel");
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
    #[test]
    fn storage_retries_but_corruption_and_cipher_errors_are_permanent() {
        let storage =
            service_job_error(SuiteStoreError::Storage("database unavailable".into()).into());
        assert!(storage.is_retryable());
        assert_eq!(storage.to_string(), "internal_error");
        for error in [
            SuiteServiceError::Cipher,
            SuiteStoreError::InvalidData("corrupt job".into()).into(),
        ] {
            let job_error = service_job_error(error);
            assert!(!job_error.is_retryable());
            assert_eq!(job_error.to_string(), "internal_error");
        }
        let disabled = service_job_error(LinkProtocolError::Disabled.into());
        assert!(!disabled.is_retryable());
        assert_eq!(disabled.to_string(), "suite_link_disabled");
    }
}
