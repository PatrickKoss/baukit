//! Error vocabulary shared by the store and runner.

/// A durable job store error.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Caller-provided data violates the store contract.
    #[error("invalid job store input: {0}")]
    InvalidInput(String),
    /// PostgreSQL could not execute an operation.
    #[error("PostgreSQL job store operation failed: {0}")]
    Database(#[source] sqlx::Error),
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
    retryable: bool,
}

impl JobError {
    /// Creates an error which may be retried while attempts remain.
    pub fn retryable(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: true,
        }
    }

    /// Creates an error which immediately moves the job to terminal failure.
    pub fn permanent(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            retryable: false,
        }
    }

    /// Returns whether the failure may be retried.
    pub const fn is_retryable(&self) -> bool {
        self.retryable
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
    /// The worker lost ownership before it could persist an attempt outcome.
    #[error("worker lost the lease for job {job_id}")]
    LeaseLost {
        /// The job whose lease no longer belongs to this worker.
        job_id: uuid::Uuid,
    },
}
