use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler};
use baukit_telemetry::tracing;

use {{ context.app_crate }}_domain::{ITEM_CREATED_JOB_TYPE, ItemCreatedJob};

const JOB_TYPES: &[&str] = &[ITEM_CREATED_JOB_TYPE];

#[derive(Clone, Default)]
pub struct DemoJobHandler;

impl DemoJobHandler {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}
{% if context.platform %}
pub mod platform {
    use std::time::Duration;

    use baukit_jobs::{
        ClaimedJob, EnqueueOutcome, FixedUtcInterval, FixedUtcSlot, JobCancellation, JobError,
        JobFuture, JobHandler, JobStore, NewJob, PostgresJobStore,
    };
    use baukit_suite::platform::PostgresPlatformInbox;
    use chrono::{DateTime, Utc};
    use serde_json::json;
    use sqlx::PgPool;

    use super::DemoJobHandler;
    use {{ context.app_crate }}_domain::ITEM_CREATED_JOB_TYPE;

    pub const PURGE_JOB_TYPE: &str = "platform.receipts.purge";
    const DAILY: Duration = Duration::from_secs(24 * 60 * 60);
    const RECEIPT_RETENTION_DAYS: i64 = 30;
    const PURGE_BATCH: u32 = 1000;
    const MAX_ATTEMPTS: u32 = 5;

    #[derive(Clone)]
    pub struct PlatformJobHandler {
        pool: PgPool,
        store: PostgresJobStore,
        interval: FixedUtcInterval,
    }

    impl PlatformJobHandler {
        pub fn new(pool: PgPool) -> Result<Self, JobError> {
            Ok(Self {
                store: PostgresJobStore::new(pool.clone()),
                pool,
                interval: FixedUtcInterval::new(DAILY)
                    .map_err(|_| JobError::permanent("invalid receipt purge interval"))?,
            })
        }

        /// Safe to call from every worker replica at startup.
        pub async fn enqueue_initial(
            &self,
            now: DateTime<Utc>,
        ) -> Result<EnqueueOutcome, JobError> {
            let slot = self
                .interval
                .slot_at(now)
                .map_err(|_| JobError::permanent("invalid receipt purge slot"))?;
            self.store
                .enqueue(purge_job(slot))
                .await
                .map_err(|_| JobError::retryable("could not enqueue receipt purge"))
        }

        async fn purge(
            &self,
            job: &ClaimedJob,
            cancellation: JobCancellation,
        ) -> Result<(), JobError> {
            let starts_at: DateTime<Utc> = serde_json::from_value(job.payload["slot"].clone())
                .map_err(|_| JobError::permanent("invalid receipt purge payload"))?;
            let slot = self
                .interval
                .slot_at(starts_at)
                .map_err(|_| JobError::permanent("invalid receipt purge slot"))?;
            if slot.starts_at() != starts_at {
                return Err(JobError::permanent("receipt purge slot is not aligned"));
            }
            let inbox = PostgresPlatformInbox::new(self.pool.clone());
            let cutoff = Utc::now() - chrono::TimeDelta::days(RECEIPT_RETENTION_DAYS);
            loop {
                if cancellation.is_cancelled() {
                    return Err(JobError::retryable("receipt purge was cancelled"));
                }
                let removed = inbox
                    .purge_received_before(cutoff, PURGE_BATCH)
                    .await
                    .map_err(|_| JobError::retryable("could not purge receipts"))?;
                if removed == 0 {
                    break;
                }
            }
            let next = self
                .interval
                .next_slot(slot, Utc::now())
                .map_err(|_| JobError::permanent("invalid next receipt purge slot"))?;
            let mut transaction = self
                .pool
                .begin()
                .await
                .map_err(|_| JobError::retryable("could not begin receipt purge completion"))?;
            self.store
                .enqueue_in_transaction(&mut transaction, purge_job(next))
                .await
                .map_err(|_| JobError::retryable("could not enqueue next receipt purge"))?;
            let completed = self
                .store
                .complete_in_transaction(
                    &mut transaction,
                    job.id,
                    cancellation.worker_id(),
                    Utc::now(),
                )
                .await
                .map_err(|_| JobError::retryable("could not complete receipt purge"))?;
            if !completed {
                return Err(JobError::retryable("receipt purge lease was lost"));
            }
            transaction
                .commit()
                .await
                .map_err(|_| JobError::retryable("could not commit receipt purge completion"))?;
            cancellation.mark_completed_in_transaction();
            Ok(())
        }
    }

    fn purge_job(slot: FixedUtcSlot) -> NewJob {
        NewJob::new(
            PURGE_JOB_TYPE,
            json!({"slot": slot.starts_at()}),
            MAX_ATTEMPTS,
        )
        .run_after(slot.starts_at())
        .idempotency_key(slot.identifier())
    }

    impl JobHandler for PlatformJobHandler {
        fn job_types(&self) -> &'static [&'static str] {
            &[ITEM_CREATED_JOB_TYPE, PURGE_JOB_TYPE]
        }

        fn handle<'a>(
            &'a self,
            job: &'a ClaimedJob,
            cancellation: JobCancellation,
        ) -> JobFuture<'a, Result<(), JobError>> {
            if job.job_type == ITEM_CREATED_JOB_TYPE {
                return DemoJobHandler.handle(job, cancellation);
            }
            Box::pin(async move {
                if job.job_type != PURGE_JOB_TYPE {
                    return Err(JobError::permanent("unsupported platform job type"));
                }
                self.purge(job, cancellation).await
            })
        }
    }
}
{% endif -%}
impl JobHandler for DemoJobHandler {
    fn job_types(&self) -> &'static [&'static str] {
        JOB_TYPES
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            if job.job_type != ITEM_CREATED_JOB_TYPE {
                return Err(JobError::permanent("unsupported demo job type"));
            }
            if cancellation.is_cancelled() {
                return Err(JobError::retryable("demo job was cancelled"));
            }
            let payload: ItemCreatedJob = serde_json::from_value(job.payload.clone())
                .map_err(|_| JobError::permanent("invalid item-created payload"))?;
            tracing::info!(
                message = "demo item-created job handled",
                job_id = %job.id,
                item_id = %payload.item_id,
            );
            Ok(())
        })
    }
}
