use std::{sync::Arc, time::Duration};

use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time,
};

use crate::ApiError;

/// Counter metric for requests shed before a handler ran, labeled `reason`.
pub const HTTP_REQUESTS_SHED_TOTAL: &str = "http_requests_shed_total";

const SHED_RETRY_AFTER: Duration = Duration::from_secs(1);
const CONCURRENCY_REASON: &str = "concurrency";

/// Admits at most `limit` requests at once and sheds the ones that wait too long.
#[derive(Clone, Debug)]
pub(crate) struct ConcurrencyGate {
    permits: Arc<Semaphore>,
    queue_timeout: Duration,
}

impl ConcurrencyGate {
    pub(crate) fn new(limit: usize, queue_timeout: Duration) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limit.min(Semaphore::MAX_PERMITS))),
            queue_timeout,
        }
    }

    pub(crate) async fn admit(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        let acquire = Arc::clone(&self.permits).acquire_owned();
        match time::timeout(self.queue_timeout, acquire).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(_)) | Err(_) => {
                tracing::warn!("request shed: no concurrency permit within the queue timeout");
                metrics::counter!(HTTP_REQUESTS_SHED_TOTAL, "reason" => CONCURRENCY_REASON)
                    .increment(1);
                Err(ApiError::unavailable(SHED_RETRY_AFTER))
            }
        }
    }
}
