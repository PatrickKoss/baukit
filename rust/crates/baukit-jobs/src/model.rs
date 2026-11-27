use jiff::Timestamp;
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
    pub run_after: Timestamp,
    /// Worker which owns a running job.
    pub locked_by: Option<String>,
    /// Expiry time for the current lease.
    pub locked_until: Option<Timestamp>,
    /// Optional key unique within `job_type`.
    pub idempotency_key: Option<String>,
    /// Key reserved while this job is pending or running, retained after completion.
    pub dedupe_key: Option<String>,
    /// Key reserved until the first claim. Claiming clears it, including for retries.
    pub pending_dedupe_key: Option<String>,
    /// Bounded diagnostic text for the most recent failure.
    pub last_error: Option<String>,
    /// Stable terminal failure reason; set only while status is `failed`.
    pub failure_reason: Option<JobFailureReason>,
    /// Time cancellation was requested for a running job.
    pub cancel_requested_at: Option<Timestamp>,
    /// Releases that returned an attempt instead of consuming it.
    pub deferrals: u32,
    /// Key shared by jobs that compete for [`crate::WorkerConfig::max_per_key`] slots.
    pub concurrency_key: Option<String>,
    /// Creation timestamp.
    pub created_at: Timestamp,
    /// Timestamp of the latest lifecycle transition.
    pub updated_at: Timestamp,
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
    pub run_after: Timestamp,
    /// Optional key unique within `job_type`.
    pub idempotency_key: Option<String>,
    /// Optional key that limits how many jobs sharing it run at once.
    pub concurrency_key: Option<String>,
    /// Creation timestamp.
    pub created_at: Timestamp,
}

impl NewJob {
    /// Creates a job ready to run immediately with a UUIDv7 identifier.
    pub fn new(job_type: impl Into<String>, payload: Value, max_attempts: u32) -> Self {
        let now = Timestamp::now();
        Self {
            id: Uuid::now_v7(),
            job_type: job_type.into(),
            payload,
            max_attempts,
            run_after: now,
            idempotency_key: None,
            concurrency_key: None,
            created_at: now,
        }
    }

    /// Sets the earliest claim time.
    #[must_use]
    pub const fn run_after(mut self, run_after: Timestamp) -> Self {
        self.run_after = run_after;
        self
    }

    /// Sets an idempotency key unique within this job type.
    #[must_use]
    pub fn idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    /// Sets the key that [`crate::WorkerConfig::max_per_key`] limits.
    ///
    /// Keys are global across job types: jobs of any type with the same key
    /// share the limit. Use one key per downstream dependency, such as a peer
    /// application, so a hanging peer holds at most `max_per_key` worker slots.
    #[must_use]
    pub fn concurrency_key(mut self, key: impl Into<String>) -> Self {
        self.concurrency_key = Some(key.into());
        self
    }
}

/// Result of an enqueue operation.
#[derive(Clone, Debug, PartialEq)]
pub struct EnqueueOutcome {
    /// The inserted job or the existing job matching the enqueue's key.
    pub job: Job,
    /// Whether this call inserted the row.
    pub created: bool,
}

/// Counts changed by cancellation inside a caller-owned transaction.
///
/// The caller must commit the transaction to make these changes durable.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CancellationOutcome {
    /// Pending jobs moved to terminal cancellation.
    pub cancelled_pending: u64,
    /// Running jobs whose cancellation request was set, including repeat requests.
    pub cancellation_requested_running: u64,
}

/// Durable disposition after recording a failed attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailureDisposition {
    /// The job returned to pending with a future `run_after` time.
    Retry,
    /// The job moved to terminal failure.
    Failed,
}

/// Why a running job goes back to pending through [`crate::JobStore::release`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseReason {
    /// The worker is shutting down. The attempt is given back and `deferrals` is unchanged.
    Shutdown,
    /// The handler deferred the job. The attempt is given back and `deferrals` grows by one.
    Deferred,
    /// The handler deferred the job past the deferral cap. The attempt stays
    /// used, so a final attempt fails with `attempts_exhausted`.
    DeferredPastCap,
}

impl ReleaseReason {
    pub(crate) const fn consumes_attempt(self) -> bool {
        matches!(self, Self::DeferredPastCap)
    }

    pub(crate) const fn counts_deferral(self) -> bool {
        matches!(self, Self::Deferred)
    }
}

/// Outbox row counts for a runner's job types, sampled for telemetry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct QueueDepth {
    /// Pending rows, including rows scheduled for later.
    pub pending: u64,
    /// Running rows, including rows whose lease has expired.
    pub running: u64,
    /// Rows in terminal failure that cleanup has not removed yet.
    pub failed: u64,
}

/// Per-status timestamps used to select terminal jobs for cleanup.
///
/// Cleanup deletes jobs whose `updated_at` is earlier than the cutoff for
/// their status. Each cutoff is independent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalJobCutoffs {
    /// Delete succeeded jobs older than this timestamp.
    pub succeeded_before: Timestamp,
    /// Delete cancelled jobs older than this timestamp.
    pub cancelled_before: Timestamp,
    /// Delete failed jobs older than this timestamp.
    pub failed_before: Timestamp,
}

/// Counts returned after one terminal-job cleanup batch commits.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TerminalJobCleanupOutcome {
    /// Succeeded jobs deleted by the batch.
    pub succeeded: u64,
    /// Cancelled jobs deleted by the batch.
    pub cancelled: u64,
    /// Failed jobs deleted by the batch.
    pub failed: u64,
}

impl TerminalJobCleanupOutcome {
    /// Returns the total number of rows deleted by the batch.
    #[must_use]
    pub const fn total(self) -> u64 {
        self.succeeded + self.cancelled + self.failed
    }
}
