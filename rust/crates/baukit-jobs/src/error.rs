//! Error vocabulary shared by the store and runner.

use std::time::Duration;

/// A durable job store error.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Caller-provided data violates the store contract.
    #[error("invalid job store input: {0}")]
    InvalidInput(String),
    /// PostgreSQL could not execute an operation.
    #[error("PostgreSQL job store operation failed: {0}")]
    Database(#[source] sqlx::Error),
    /// An owned store operation panicked or was aborted by the runtime.
    #[error("job store task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    /// Stored data does not satisfy Baukit's model.
    #[error("invalid job outbox data: {0}")]
    InvalidData(String),
}

impl StoreError {
    pub(crate) fn database(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

/// An error returned by product job handling.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct JobError {
    message: String,
    kind: JobErrorKind,
}

#[derive(Clone, Copy, Debug)]
enum JobErrorKind {
    Retryable { retry_after: Option<Duration> },
    Permanent,
    Deferred { delay: Duration },
}

impl JobError {
    /// Creates an error which may be retried while attempts remain.
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: JobErrorKind::Retryable { retry_after: None },
        }
    }

    /// Creates a retryable error with a provider-directed delay.
    pub fn retryable_after(message: impl Into<String>, retry_after: Duration) -> Self {
        Self {
            message: message.into(),
            kind: JobErrorKind::Retryable {
                retry_after: Some(retry_after),
            },
        }
    }

    /// Creates an error which immediately moves the job to terminal failure.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: JobErrorKind::Permanent,
        }
    }

    /// Returns the job to pending after `delay` without consuming an attempt.
    ///
    /// Use this when a dependency refuses work for a known period, such as an
    /// open circuit breaker. Once a job has used `WorkerConfig::max_deferrals`
    /// free deferrals, a further deferral consumes an attempt like
    /// [`Self::retryable_after`].
    pub fn deferred(message: impl Into<String>, delay: Duration) -> Self {
        Self {
            message: message.into(),
            kind: JobErrorKind::Deferred { delay },
        }
    }

    /// Returns whether the failure may be retried. Deferrals are retryable.
    pub const fn is_retryable(&self) -> bool {
        !matches!(self.kind, JobErrorKind::Permanent)
    }

    /// Returns the provider-directed retry delay, when supplied.
    ///
    /// For a deferral this is the deferral delay.
    pub const fn retry_after(&self) -> Option<Duration> {
        match self.kind {
            JobErrorKind::Retryable { retry_after } => retry_after,
            JobErrorKind::Permanent => None,
            JobErrorKind::Deferred { delay } => Some(delay),
        }
    }

    /// Returns the deferral delay when this error came from [`Self::deferred`].
    pub const fn deferral(&self) -> Option<Duration> {
        match self.kind {
            JobErrorKind::Deferred { delay } => Some(delay),
            JobErrorKind::Retryable { .. } | JobErrorKind::Permanent => None,
        }
    }
}

/// A worker loop failure which should be surfaced to runtime supervision.
#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    /// A durable store operation failed.
    #[error(transparent)]
    Store(#[from] StoreError),
    /// A spawned attempt panicked or was externally aborted.
    #[error("worker attempt task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    /// The configured worker settings are invalid.
    #[error("invalid worker configuration: {0}")]
    InvalidConfig(String),
}
