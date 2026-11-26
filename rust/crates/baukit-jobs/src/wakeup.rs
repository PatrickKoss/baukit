use std::sync::Arc;

use tokio::sync::Notify;

/// Wakes an idle [`WorkerRunner`](crate::WorkerRunner) before its next poll.
///
/// Clones share one signal. A wake that arrives while the runner is busy is
/// kept until the runner next waits, so it is never lost. Polling continues
/// without wake-ups, so a wake source that fails only costs latency.
#[derive(Clone, Debug, Default)]
pub struct JobWakeup(Arc<Notify>);

impl JobWakeup {
    /// Creates an unconnected wake-up signal.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the runner to claim now instead of after its idle delay.
    pub fn wake(&self) {
        self.0.notify_one();
    }

    pub(crate) async fn woken(&self) {
        self.0.notified().await;
    }
}
