use super::*;

pub struct SuiteErasureNotificationService {
    links: Arc<dyn SuiteErasureLinks>,
    revoker: Arc<dyn SuiteProfileRevoker>,
}

impl SuiteErasureNotificationService {
    pub fn new(links: Arc<dyn SuiteErasureLinks>, revoker: Arc<dyn SuiteProfileRevoker>) -> Self {
        Self { links, revoker }
    }
}

#[async_trait]
impl SuiteErasureNotifier for SuiteErasureNotificationService {
    async fn notify_before_erasure(&self, subject: &str) {
        match self.links.links_for_erasure(subject).await {
            Ok(links) => self.revoker.revoke_before_erasure(&links).await,
            Err(error) => {
                let error = SuiteServiceError::from(error);
                tracing::warn!(code = error.code(), "Suite erasure link lookup failed");
            }
        }
    }
}

/// Checks erasure admission, then sends bounded revokes before the product transaction.
/// Replays and rejected requests return without notifying peers.
///
/// Two concurrent first requests with the same key may both notify for the same
/// current profile, which is the profile being erased. `erase` repeats all checks
/// under its locks after notification.
pub async fn erase_with_suite(
    service: &baukit_erasure::ErasureService,
    notifier: &dyn SuiteErasureNotifier,
    subject: &str,
    authenticated_at: Option<chrono::DateTime<chrono::Utc>>,
    key: &str,
    product: &dyn baukit_erasure::ProductErasure,
) -> Result<baukit_erasure::ErasureOutcome, baukit_erasure::ErasureError> {
    if let baukit_erasure::ErasurePreflight::Replay(outcome) =
        service.preflight(subject, authenticated_at, key).await?
    {
        return Ok(outcome);
    }
    notifier.notify_before_erasure(subject).await;
    service.erase(subject, authenticated_at, key, product).await
}
