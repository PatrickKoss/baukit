use crate::{JobFailureReason, JobStatus, NewJob, StoreError};

use super::{
    MAX_DEDUPE_KEY_LENGTH, MAX_ERROR_LENGTH, MAX_IDEMPOTENCY_KEY_LENGTH, MAX_JOB_TYPE_LENGTH,
    MAX_WORKER_ID_LENGTH,
};

pub(super) fn validate_deduplicated_job(job: &NewJob, dedupe_key: &str) -> Result<(), StoreError> {
    validate_new_job(job)?;
    validate_text("dedupe_key", dedupe_key, MAX_DEDUPE_KEY_LENGTH)?;
    if job.idempotency_key.is_some() {
        return Err(StoreError::InvalidInput(
            "job deduplication cannot be combined with an idempotency key".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn validate_new_job(job: &NewJob) -> Result<(), StoreError> {
    validate_text("job_type", &job.job_type, MAX_JOB_TYPE_LENGTH)?;
    if job.max_attempts == 0 {
        return Err(StoreError::InvalidInput(
            "max_attempts must be greater than zero".to_owned(),
        ));
    }
    if let Some(key) = &job.idempotency_key {
        validate_text("idempotency_key", key, MAX_IDEMPOTENCY_KEY_LENGTH)?;
    }
    Ok(())
}

pub(super) fn validate_job_types(job_types: &[&str]) -> Result<(), StoreError> {
    if job_types.is_empty() {
        return Err(StoreError::InvalidInput(
            "job_types must contain at least one job type".to_owned(),
        ));
    }
    job_types
        .iter()
        .try_for_each(|job_type| validate_text("job_type", job_type, MAX_JOB_TYPE_LENGTH))
}

pub(super) fn validate_worker_id(worker_id: &str) -> Result<(), StoreError> {
    validate_text("worker_id", worker_id, MAX_WORKER_ID_LENGTH)
}

pub(super) fn validate_text(name: &str, value: &str, max: usize) -> Result<(), StoreError> {
    let length = value.chars().count();
    if value.trim().is_empty() || length > max {
        Err(StoreError::InvalidInput(format!(
            "{name} must contain 1 to {max} characters"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn bound_error(error: &str) -> String {
    let bounded = error.trim();
    if bounded.is_empty() {
        "job failed without an error message".to_owned()
    } else {
        bounded.chars().take(MAX_ERROR_LENGTH).collect()
    }
}

pub(super) fn status_name(status: JobStatus) -> &'static str {
    match status {
        JobStatus::Pending => "pending",
        JobStatus::Running => "running",
        JobStatus::Succeeded => "succeeded",
        JobStatus::Failed => "failed",
        JobStatus::Cancelled => "cancelled",
    }
}

pub(super) fn parse_status(status: &str) -> Result<JobStatus, StoreError> {
    match status {
        "pending" => Ok(JobStatus::Pending),
        "running" => Ok(JobStatus::Running),
        "succeeded" => Ok(JobStatus::Succeeded),
        "failed" => Ok(JobStatus::Failed),
        "cancelled" => Ok(JobStatus::Cancelled),
        value => Err(StoreError::InvalidData(format!(
            "unknown job status `{value}`"
        ))),
    }
}

pub(super) fn parse_failure_reason(reason: &str) -> Result<JobFailureReason, StoreError> {
    match reason {
        "permanent" => Ok(JobFailureReason::Permanent),
        "attempts_exhausted" => Ok(JobFailureReason::AttemptsExhausted),
        value => Err(StoreError::InvalidData(format!(
            "unknown job failure reason `{value}`"
        ))),
    }
}
