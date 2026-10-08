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
        SuiteServiceError::Cipher
        | SuiteServiceError::PayloadInvalid
        | SuiteServiceError::Store(
            SuiteStoreError::InvalidData(_) | SuiteStoreError::PayloadInvalid(_),
        ) => JobError::permanent(error.code()),
        _ => JobError::retryable(error.code()),
    }
}

/// Runs cleanup on UTC hour boundaries until the shutdown channel closes or becomes true.
/// Storage failures are logged and counted, then retried at the next boundary.
pub async fn run_hourly_cleanup(
    service: Arc<dyn SuiteDeliveryRunner>,
    shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), SuiteServiceError> {
    run_hourly_cleanup_with_clock(service, shutdown, Utc::now).await
}

async fn run_hourly_cleanup_with_clock(
    service: Arc<dyn SuiteDeliveryRunner>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
    now: impl Fn() -> chrono::DateTime<Utc>,
) -> Result<(), SuiteServiceError> {
    metrics::describe_counter!(
        "suite_cleanup_failures_total",
        "Failed hourly suite cleanup runs"
    );
    metrics::counter!("suite_cleanup_failures_total").increment(0);
    let interval = baukit_jobs::FixedUtcInterval::new(std::time::Duration::from_secs(60 * 60))
        .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
    let mut slot = interval
        .slot_at(now())
        .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
    loop {
        if *shutdown.borrow() {
            return Ok(());
        }
        slot = interval
            .next_slot(slot, now())
            .map_err(|error| SuiteStoreError::InvalidData(error.to_string()))?;
        let wait = (slot.starts_at() - now()).to_std().unwrap_or_default();
        tokio::select! {
            () = shutdown_requested(&mut shutdown) => { return Ok(()); }
            () = tokio::time::sleep(wait) => {
                if let Err(error) = service.cleanup(now()).await {
                    tracing::warn!(code = error.code(), error = %error, "Suite cleanup failed; retrying next hour");
                    metrics::counter!("suite_cleanup_failures_total").increment(1);
                }
            }
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
    struct CleanupMetrics(Arc<std::sync::atomic::AtomicU64>);
    impl metrics::Recorder for CleanupMetrics {
        fn describe_counter(
            &self,
            key: metrics::KeyName,
            unit: Option<metrics::Unit>,
            description: metrics::SharedString,
        ) {
            metrics::NoopRecorder.describe_counter(key, unit, description);
        }
        fn describe_gauge(
            &self,
            key: metrics::KeyName,
            unit: Option<metrics::Unit>,
            description: metrics::SharedString,
        ) {
            metrics::NoopRecorder.describe_gauge(key, unit, description);
        }
        fn describe_histogram(
            &self,
            key: metrics::KeyName,
            unit: Option<metrics::Unit>,
            description: metrics::SharedString,
        ) {
            metrics::NoopRecorder.describe_histogram(key, unit, description);
        }
        fn register_counter(
            &self,
            key: &metrics::Key,
            _: &metrics::Metadata<'_>,
        ) -> metrics::Counter {
            assert_eq!(key.name(), "suite_cleanup_failures_total");
            assert_eq!(key.labels().count(), 0);
            metrics::Counter::from_arc(self.0.clone())
        }
        fn register_gauge(
            &self,
            key: &metrics::Key,
            metadata: &metrics::Metadata<'_>,
        ) -> metrics::Gauge {
            metrics::NoopRecorder.register_gauge(key, metadata)
        }
        fn register_histogram(
            &self,
            key: &metrics::Key,
            metadata: &metrics::Metadata<'_>,
        ) -> metrics::Histogram {
            metrics::NoopRecorder.register_histogram(key, metadata)
        }
    }
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
            if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                return Err(SuiteStoreError::Storage("cleanup database unavailable".into()).into());
            }
            Ok(SuiteCleanupOutcome::default())
        }
    }
    #[tokio::test(start_paused = true)]
    async fn cleanup_survives_storage_failure_and_retries_at_the_next_boundary() {
        let failures = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let recorder = CleanupMetrics(failures.clone());
        let _guard = metrics::set_default_local_recorder(&recorder);
        let service = Arc::new(CleanupRunner(std::sync::atomic::AtomicUsize::new(0)));
        let (sender, receiver) = tokio::sync::watch::channel(false);
        let started = tokio::time::Instant::now();
        let anchor: chrono::DateTime<Utc> = "2026-10-08T12:30:00Z".parse().expect("time");
        let task = tokio::spawn(run_hourly_cleanup_with_clock(
            service.clone(),
            receiver,
            move || anchor + chrono::Duration::from_std(started.elapsed()).expect("elapsed"),
        ));
        tokio::task::yield_now().await;
        sender.send(false).expect("notification");
        tokio::time::advance(std::time::Duration::from_secs(1799)).await;
        tokio::task::yield_now().await;
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 0);
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        tokio::time::advance(std::time::Duration::from_secs(3599)).await;
        tokio::task::yield_now().await;
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 1);
        tokio::time::advance(std::time::Duration::from_secs(1)).await;
        tokio::task::yield_now().await;
        assert_eq!(service.0.load(std::sync::atomic::Ordering::SeqCst), 2);
        sender.send(true).expect("shutdown");
        task.await.expect("task").expect("shutdown after failures");
        assert_eq!(failures.load(std::sync::atomic::Ordering::SeqCst), 1);
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
        let payload = service_job_error(
            SuiteStoreError::PayloadInvalid(PayloadError::InvalidPayload("typed payload".into()))
                .into(),
        );
        assert!(!payload.is_retryable());
        assert_eq!(payload.to_string(), "suite_payload_invalid");
        let disabled = service_job_error(LinkProtocolError::Disabled.into());
        assert!(!disabled.is_retryable());
        assert_eq!(disabled.to_string(), "suite_link_disabled");
    }
}
