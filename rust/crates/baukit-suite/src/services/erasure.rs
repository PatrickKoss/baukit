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

/// Sends bounded revokes before baukit-erasure opens its product transaction.
pub async fn erase_with_suite(
    service: &baukit_erasure::ErasureService,
    notifier: &dyn SuiteErasureNotifier,
    subject: &str,
    key: &str,
    product: &dyn baukit_erasure::ProductErasure,
) -> Result<baukit_erasure::ErasureOutcome, baukit_erasure::ErasureError> {
    notifier.notify_before_erasure(subject).await;
    service.erase(subject, key, product).await
}
