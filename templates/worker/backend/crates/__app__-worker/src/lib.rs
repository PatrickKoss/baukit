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
