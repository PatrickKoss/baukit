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
{% if context.platform or context.auth_enabled %}
pub mod retention {
    use std::{num::NonZeroU32, time::Duration};

    use baukit_jobs::{
        ClaimedJob, EnqueueOutcome, FixedUtcInterval, JobCancellation, JobError, JobFuture,
        JobHandler, PostgresJobStore, RecurringJob,
    };
{% if context.platform %}    use baukit_suite::platform::PostgresPlatformInbox;
{% endif %}    use jiff::Timestamp;
    use sqlx::PgPool;

    use super::DemoJobHandler;
    use {{ context.app_crate }}_domain::ITEM_CREATED_JOB_TYPE;

    pub const PURGE_JOB_TYPE: &str = "retention.purge";
    const DAILY: Duration = Duration::from_secs(24 * 60 * 60);
{% if context.platform %}    const RECEIPT_RETENTION_DAYS: i64 = 30;
{% endif %}{% if context.auth_enabled %}    const BROWSER_SESSION_GRACE_DAYS: i64 = 1;
{% endif %}    const PURGE_BATCH: u32 = 1000;
    const MAX_ATTEMPTS: u32 = 5;

    #[derive(Clone)]
    pub struct RetentionJobHandler {
        pool: PgPool,
        store: PostgresJobStore,
        recurring: RecurringJob,
    }

    impl RetentionJobHandler {
        pub fn new(pool: PgPool) -> Result<Self, JobError> {
            Ok(Self {
                store: PostgresJobStore::new(pool.clone()),
                pool,
                recurring: RecurringJob::new(
                    PURGE_JOB_TYPE,
                    FixedUtcInterval::new(DAILY)
                        .map_err(|_| JobError::permanent("invalid retention purge interval"))?,
                    MAX_ATTEMPTS,
                ),
            })
        }

        /// Register with `WorkerRunner::with_recurring`, so a slot that fails
        /// for good still schedules the next one.
        #[must_use]
        pub const fn recurring(&self) -> RecurringJob {
            self.recurring
        }

        /// Safe to call from every worker replica at startup.
        pub async fn enqueue_initial(&self, now: Timestamp) -> Result<EnqueueOutcome, JobError> {
            self.recurring
                .enqueue_initial(&self.store, now)
                .await
                .map_err(|_| JobError::retryable("could not enqueue retention purge"))
        }

        async fn purge_expired(&self, cancellation: &JobCancellation) -> Result<(), JobError> {
{% if context.platform %}            self.purge_receipts(cancellation).await?;
{% endif %}{% if context.auth_enabled %}            self.purge_browser_sessions(cancellation).await?;
{% endif %}            Ok(())
        }
{% if context.platform %}
        async fn purge_receipts(&self, cancellation: &JobCancellation) -> Result<(), JobError> {
            let inbox = PostgresPlatformInbox::new(self.pool.clone());
            let cutoff =
                Timestamp::now() - jiff::SignedDuration::from_hours(24 * RECEIPT_RETENTION_DAYS);
            loop {
                if cancellation.is_cancelled() {
                    return Err(JobError::retryable("retention purge was cancelled"));
                }
                let removed = inbox
                    .purge_received_before(cutoff, PURGE_BATCH)
                    .await
                    .map_err(|_| JobError::retryable("could not purge receipts"))?;
                if removed == 0 {
                    return Ok(());
                }
            }
        }
{% endif %}{% if context.auth_enabled %}
        async fn purge_browser_sessions(
            &self,
            cancellation: &JobCancellation,
        ) -> Result<(), JobError> {
            let cutoff = Timestamp::now()
                - jiff::SignedDuration::from_hours(24 * BROWSER_SESSION_GRACE_DAYS);
            let batch = NonZeroU32::new(PURGE_BATCH)
                .ok_or_else(|| JobError::permanent("purge batch must not be zero"))?;
            loop {
                if cancellation.is_cancelled() {
                    return Err(JobError::retryable("retention purge was cancelled"));
                }
                let removed =
                    baukit_auth::browser_session::purge_browser_sessions(&self.pool, cutoff, batch)
                        .await
                        .map_err(|_| JobError::retryable("could not purge browser sessions"))?;
                if removed == 0 {
                    return Ok(());
                }
            }
        }
{% endif %}
        async fn purge(
            &self,
            job: &ClaimedJob,
            cancellation: JobCancellation,
        ) -> Result<(), JobError> {
            self.recurring
                .slot_of(job)
                .map_err(|_| JobError::permanent("invalid retention purge payload"))?;
            self.purge_expired(&cancellation).await?;
            let next = self
                .recurring
                .next_job(job, Timestamp::now())
                .map_err(|_| JobError::permanent("invalid next retention purge slot"))?;
            let mut transaction = baukit_ops::begin(&self.pool)
                .await
                .map_err(|_| JobError::retryable("could not begin retention purge completion"))?;
            self.store
                .enqueue_in_transaction(&mut transaction, next)
                .await
                .map_err(|_| JobError::retryable("could not enqueue next retention purge"))?;
            let completed = self
                .store
                .complete_lease_in_transaction(
                    &mut transaction,
                    cancellation.lease(),
                    Timestamp::now(),
                )
                .await
                .map_err(|_| JobError::retryable("could not complete retention purge"))?;
            if !completed {
                return Err(JobError::retryable("retention purge lease was lost"));
            }
            transaction
                .commit()
                .await
                .map_err(|_| JobError::retryable("could not commit retention purge completion"))?;
            cancellation.mark_completed_in_transaction();
            Ok(())
        }
    }

    impl JobHandler for RetentionJobHandler {
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
