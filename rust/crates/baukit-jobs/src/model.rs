use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Durable lifecycle state for an outbox job.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    /// Waiting until `run_after` and available for claim.
    Pending,
    /// Owned by a worker until its lease expires.
    Running,
    /// Completed successfully.
    Succeeded,
    /// Exhausted attempts or encountered a permanent failure.
    Failed,
    /// Cancelled before or during execution.
    Cancelled,
}

/// Stable reason a job entered terminal [`JobStatus::Failed`].
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobFailureReason {
    /// The handler classified the failure as non-retryable.
    Permanent,
    /// A retryable failure or expired lease consumed the final attempt.
    AttemptsExhausted,
}

/// A job persisted in the product's outbox table.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Job {
    /// Stable job identifier.
    pub id: Uuid,
    /// Static product-defined dispatch identifier.
    pub job_type: String,
    /// Product-defined JSON payload. Payload fields are never metric labels.
    pub payload: Value,
    /// Current lifecycle state.
    pub status: JobStatus,
    /// Number of successful claims, including the current attempt.
    pub attempts: u32,
    /// Maximum number of claims before terminal failure.
    pub max_attempts: u32,
    /// Earliest time a pending job may be claimed.
    pub run_after: DateTime<Utc>,
    /// Worker which owns a running job.
    pub locked_by: Option<String>,
    /// Expiry time for the current lease.
    pub locked_until: Option<DateTime<Utc>>,
    /// Optional key unique within `job_type`.
    pub idempotency_key: Option<String>,
    /// Bounded diagnostic text for the most recent failure.
    pub last_error: Option<String>,
    /// Stable terminal failure reason; set only while status is `failed`.
    pub failure_reason: Option<JobFailureReason>,
    /// Time cancellation was requested for a running job.
    pub cancel_requested_at: Option<DateTime<Utc>>,
    /// Creation timestamp used for queue-age telemetry.
    pub created_at: DateTime<Utc>,
    /// Timestamp of the latest lifecycle transition.
    pub updated_at: DateTime<Utc>,
}

/// A job returned from a successful claim.
pub type ClaimedJob = Job;

/// Input for enqueueing a durable job.
#[derive(Clone, Debug, Serialize)]
pub struct NewJob {
    /// Stable job identifier supplied by the producer.
    pub id: Uuid,
    /// Static product-defined dispatch identifier.
    pub job_type: String,
    /// Product-defined JSON payload.
    pub payload: Value,
    /// Maximum number of claims before terminal failure.
    pub max_attempts: u32,
    /// Earliest time the job may be claimed.
    pub run_after: DateTime<Utc>,
    /// Optional key unique within `job_type`.
    pub idempotency_key: Option<String>,
    /// Creation timestamp.
    pub created_at: DateTime<Utc>,
}

impl NewJob {
    /// Creates a job ready to run immediately with a UUIDv7 identifier.
    pub fn new(job_type: impl Into<String>, payload: Value, max_attempts: u32) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::now_v7(),
            job_type: job_type.into(),
            payload,
            max_attempts,
            run_after: now,
            idempotency_key: None,
            created_at: now,
        }
    }

    /// Sets the earliest claim time.
    pub const fn run_after(mut self, run_after: DateTime<Utc>) -> Self {
        self.run_after = run_after;
        self
    }

    /// Sets an idempotency key unique within this job type.
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }
}

/// Result of an idempotent enqueue operation.
#[derive(Clone, Debug, PartialEq)]
pub struct EnqueueOutcome {
    /// The inserted job or the previously inserted idempotent equivalent.
    pub job: Job,
    /// Whether this call inserted the row.
    pub created: bool,
}

/// Durable disposition after recording a failed attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    /// The job returned to pending with a future `run_after` time.
    Retry,
    /// The job moved to terminal failure.
    Failed,
}
