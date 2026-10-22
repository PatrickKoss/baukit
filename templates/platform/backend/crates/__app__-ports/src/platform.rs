use baukit_suite::platform::{EventContext, PlatformError, PlatformEvent};

use crate::PortFuture;

/// Product policy for hub events. Use the receipt transaction for product writes.
pub trait PlatformEvents: Send + Sync {
    fn handle<'a>(
        &'a self,
        event: &'a PlatformEvent,
        context: &'a mut EventContext<'_>,
    ) -> PortFuture<'a, Result<(), PlatformError>>;
}
