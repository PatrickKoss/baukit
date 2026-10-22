use super::*;
use async_trait::async_trait;
use std::collections::BTreeMap;
use tokio::sync::Mutex;

/// Test adapter. Receipts last for the adapter's lifetime. A cancelled or failed
/// handler leaves no receipt. Product memory side effects must tolerate replay.
#[derive(Default)]
pub struct InMemoryPlatformInbox {
    receipts: Mutex<BTreeMap<String, [u8; 32]>>,
}
#[async_trait]
impl PlatformInbox for InMemoryPlatformInbox {
    async fn receive(
        &self,
        event: &PlatformEvent,
        body_hash: [u8; 32],
        handler: &dyn PlatformEventHandler,
    ) -> Result<ReceiptOutcome, PlatformError> {
        let mut receipts = self.receipts.lock().await;
        if let Some(previous) = receipts.get(&event.envelope().event_id) {
            return if *previous == body_hash {
                Ok(ReceiptOutcome::Duplicate)
            } else {
                Err(PlatformError::EventIdConflict)
            };
        }
        if !matches!(event.kind(), PlatformEventKind::Unknown) {
            handler.handle(event, &mut EventContext::memory()).await?;
        }
        receipts.insert(event.envelope().event_id.clone(), body_hash);
        Ok(ReceiptOutcome::Applied)
    }
}
