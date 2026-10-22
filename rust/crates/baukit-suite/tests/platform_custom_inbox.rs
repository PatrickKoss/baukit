#![cfg(feature = "platform")]

use async_trait::async_trait;
use baukit_suite::platform::*;
use chrono::{DateTime, Utc};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Mutex;

#[derive(Default)]
struct ProductInbox {
    receipts: Mutex<BTreeMap<String, [u8; 32]>>,
}
#[async_trait]
impl PlatformInbox for ProductInbox {
    async fn receive(
        &self,
        event: &PlatformEvent,
        hash: [u8; 32],
        handler: &dyn PlatformEventHandler,
    ) -> Result<ReceiptOutcome, PlatformError> {
        let known = !matches!(event.kind(), PlatformEventKind::Unknown);
        let prepared = if known {
            handler.prepare(event).await?
        } else {
            None
        };
        let handler = prepared.as_deref().unwrap_or(handler);
        let mut receipts = self.receipts.lock().await;
        if let Some(previous) = receipts.get(&event.envelope().event_id) {
            return if *previous == hash {
                Ok(ReceiptOutcome::Duplicate)
            } else {
                Err(PlatformError::EventIdConflict)
            };
        }
        if known {
            handler.handle(event, &mut EventContext::detached()).await?;
        }
        receipts.insert(event.envelope().event_id.clone(), hash);
        Ok(ReceiptOutcome::Applied)
    }
}
struct ProductHandler(Arc<AtomicUsize>);
#[async_trait]
impl PlatformEventHandler for ProductHandler {
    async fn handle(
        &self,
        _: &PlatformEvent,
        _: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[tokio::test]
async fn a_product_can_implement_the_inbox_and_invoke_a_handler() {
    let fixture: Vec<serde_json::Value> = serde_json::from_str(include_str!(
        "../../../../fixtures/platform-events/v1/envelopes.json"
    ))
    .expect("fixture");
    let source = fixture[0]["sourceApp"].as_str().expect("source");
    let decoder = PlatformEventDecoder::new(source.into(), None).expect("decoder");
    let event = decoder
        .decode(
            &serde_json::to_vec(&fixture[0]).expect("json"),
            fixture[0]["occurredAt"]
                .as_str()
                .expect("time")
                .parse::<DateTime<Utc>>()
                .expect("clock"),
        )
        .expect("event");
    let inbox = ProductInbox::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let handler = ProductHandler(calls.clone());
    assert_eq!(
        inbox.receive(&event, [1; 32], &handler).await,
        Ok(ReceiptOutcome::Applied)
    );
    assert_eq!(
        inbox.receive(&event, [1; 32], &handler).await,
        Ok(ReceiptOutcome::Duplicate)
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
