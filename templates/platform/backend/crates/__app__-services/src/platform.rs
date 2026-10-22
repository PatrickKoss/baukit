use baukit_telemetry::tracing;
use {{ context.app_crate }}_ports::{PlatformEvents, PortFuture};

use baukit_suite::platform::{EventContext, PlatformError, PlatformEvent, PlatformEventKind};

/// Replace with product policy before enabling hub erasure deliveries.
pub struct LoggingPlatformEvents;

impl PlatformEvents for LoggingPlatformEvents {
    fn handle<'a>(
        &'a self,
        event: &'a PlatformEvent,
        _context: &'a mut EventContext<'_>,
    ) -> PortFuture<'a, Result<(), PlatformError>> {
        Box::pin(async move {
            if matches!(event.kind(), PlatformEventKind::ErasureRequested(_)) {
                tracing::warn!(message = "platform erasure requires a product handler");
                return Err(PlatformError::Unavailable);
            }
            tracing::info!(event_type = %event.envelope().event_type, message = "platform event received");
            Ok(())
        })
    }
}
