use std::time::Duration;

use baukit_resilience::{Backoff, SplitMixJitter};
use baukit_runtime::ShutdownToken;
use sqlx::{PgPool, postgres::PgListener};
use tokio::task::JoinHandle;

use crate::JobWakeup;

/// Channel the store notifies on every inserted or requeued job.
pub const JOB_ENQUEUED_CHANNEL: &str = "job_enqueued";

const RECONNECT_INITIAL: Duration = Duration::from_millis(250);
const RECONNECT_MAX: Duration = Duration::from_secs(30);

/// Spawns a task that turns `NOTIFY job_enqueued` into [`JobWakeup::wake`].
///
/// The task holds one connection from `pool` outside the pool for as long as
/// it listens. Behind a transaction-mode pooler, pass a pool that connects to
/// PostgreSQL directly. A lost connection wakes the runner once, because
/// notifications sent meanwhile are gone, and reconnects with jittered
/// backoff capped at 30 s. The task ends when `shutdown` begins.
pub fn spawn_enqueue_listener(
    pool: PgPool,
    wakeup: JobWakeup,
    shutdown: ShutdownToken,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        tokio::select! {
            () = shutdown.cancelled() => {}
            () = listen_forever(&pool, &wakeup) => {}
        }
    })
}

async fn listen_forever(pool: &PgPool, wakeup: &JobWakeup) {
    let backoff = Backoff::new(RECONNECT_INITIAL, RECONNECT_MAX);
    let mut jitter = SplitMixJitter::from_entropy();
    let mut failures = 0_u32;
    loop {
        if let Err(error) = listen_until_lost(pool, wakeup, &mut failures).await {
            tracing::warn!(%error, "job enqueue listener failed; polling continues");
        }
        wakeup.wake();
        let delay = backoff.next_delay(failures, &mut jitter);
        failures = failures.saturating_add(1);
        tokio::time::sleep(delay).await;
    }
}

/// Returns when the connection drops or fails. Every notification resets
/// `failures`, so a healthy listener reconnects fast after a drop.
async fn listen_until_lost(
    pool: &PgPool,
    wakeup: &JobWakeup,
    failures: &mut u32,
) -> Result<(), sqlx::Error> {
    let mut listener = PgListener::connect_with(pool).await?;
    listener.eager_reconnect(false);
    listener.listen(JOB_ENQUEUED_CHANNEL).await?;
    while listener.try_recv().await?.is_some() {
        *failures = 0;
        wakeup.wake();
    }
    Ok(())
}

/// Sends the wake-up inside the caller's transaction, so listeners hear it
/// only after the row commits.
pub(super) async fn notify_enqueued(
    connection: &mut sqlx::PgConnection,
    job_type: &str,
) -> Result<(), crate::StoreError> {
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind(JOB_ENQUEUED_CHANNEL)
        .bind(job_type)
        .execute(connection)
        .await?;
    Ok(())
}
