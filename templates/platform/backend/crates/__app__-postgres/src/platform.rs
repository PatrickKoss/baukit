use std::sync::Arc;

use async_trait::async_trait;
use baukit_suite::platform::{EventContext, PlatformError, PlatformEvent, PlatformEventHandler};
use {{ context.app_crate }}_ports::PlatformEvents;

pub struct ProductPlatformHandler(pub Arc<dyn PlatformEvents>);

#[async_trait]
impl PlatformEventHandler for ProductPlatformHandler {
    async fn handle(
        &self,
        event: &PlatformEvent,
        context: &mut EventContext<'_>,
    ) -> Result<(), PlatformError> {
        self.0.handle(event, context).await
    }
}
