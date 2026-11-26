use std::time::Duration;

use crate::RunnerError;

use super::WorkerConfig;

const MIN_LEASE_DURATION: Duration = Duration::from_secs(1);

pub(super) fn validate_config(
    config: &WorkerConfig,
    job_types: &'static [&'static str],
) -> Result<(), RunnerError> {
    if config.worker_id.trim().is_empty() {
        return Err(RunnerError::InvalidConfig(
            "worker_id must not be empty".to_owned(),
        ));
    }
    if config.queue.trim().is_empty() {
        return Err(RunnerError::InvalidConfig(
            "queue must not be empty".to_owned(),
        ));
    }
    if config.concurrency == 0 {
        return Err(RunnerError::InvalidConfig(
            "concurrency must be greater than zero".to_owned(),
        ));
    }
    for (name, duration) in [
        ("poll_interval", config.poll_interval),
        ("lease_duration", config.lease_duration),
        ("job_timeout", config.job_timeout),
        (
            "cancellation_poll_interval",
            config.cancellation_poll_interval,
        ),
        ("retry_initial", config.retry_initial),
        ("retry_max", config.retry_max),
        ("queue_sample_interval", config.queue_sample_interval),
        ("store_call_timeout", config.store_call_timeout),
    ] {
        if duration.is_zero() {
            return Err(RunnerError::InvalidConfig(format!(
                "{name} must be greater than zero"
            )));
        }
    }
    if config.retry_initial > config.retry_max {
        return Err(RunnerError::InvalidConfig(
            "retry_initial must not exceed retry_max".to_owned(),
        ));
    }
    if config.lease_duration < MIN_LEASE_DURATION {
        return Err(RunnerError::InvalidConfig(
            "lease_duration must be at least one second".to_owned(),
        ));
    }
    if job_types.is_empty() || job_types.iter().any(|job| job.trim().is_empty()) {
        return Err(RunnerError::InvalidConfig(
            "job_types must contain non-empty static identifiers".to_owned(),
        ));
    }
    for (index, job) in job_types.iter().enumerate() {
        if job_types.iter().take(index).any(|earlier| earlier == job) {
            return Err(RunnerError::InvalidConfig(format!(
                "duplicate job type `{job}`"
            )));
        }
    }
    Ok(())
}
