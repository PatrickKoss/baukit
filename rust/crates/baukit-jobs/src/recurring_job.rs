use jiff::Timestamp;
use serde_json::json;
use thiserror::Error;

use crate::{
    EnqueueOutcome, FixedUtcInterval, FixedUtcIntervalError, FixedUtcSlot, Job, JobStatus,
    JobStore, NewJob, StoreError,
};

const SLOT_FIELD: &str = "slot";

/// A job type that runs once per [`FixedUtcInterval`] slot.
///
/// Each slot is one row with payload `{"slot": <slot start>}`, `run_after` at
/// the slot start and the slot's [`FixedUtcSlot::identifier`] as idempotency
/// key, so every replica can enqueue the same slot safely. The handler
/// enqueues [`Self::next_job`] when a slot succeeds. Register the job with
/// [`WorkerRunner::with_recurring`](crate::WorkerRunner::with_recurring) so
/// the runner enqueues the next slot when one fails for good.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecurringJob {
    job_type: &'static str,
    interval: FixedUtcInterval,
    max_attempts: u32,
}

impl RecurringJob {
    /// Describes a recurring job type and the attempt budget of each slot.
    #[must_use]
    pub const fn new(
        job_type: &'static str,
        interval: FixedUtcInterval,
        max_attempts: u32,
    ) -> Self {
        Self {
            job_type,
            interval,
            max_attempts,
        }
    }

    /// Returns the static job type.
    #[must_use]
    pub const fn job_type(&self) -> &'static str {
        self.job_type
    }

    /// Returns the row for `slot`.
    #[must_use]
    pub fn job_for(&self, slot: FixedUtcSlot) -> NewJob {
        NewJob::new(
            self.job_type,
            json!({ SLOT_FIELD: slot.starts_at() }),
            self.max_attempts,
        )
        .run_after(slot.starts_at())
        .idempotency_key(slot.identifier())
    }

    /// Reads the slot a claimed row of this type runs.
    ///
    /// # Errors
    ///
    /// [`RecurringJobError::InvalidPayload`] when the row is of another type, has
    /// no slot, or its slot is not aligned to the interval, and
    /// [`RecurringJobError::Interval`] when the slot is out of range.
    pub fn slot_of(&self, job: &Job) -> Result<FixedUtcSlot, RecurringJobError> {
        if job.job_type != self.job_type {
            return Err(RecurringJobError::InvalidPayload);
        }
        let starts_at: Timestamp = job
            .payload
            .get(SLOT_FIELD)
            .cloned()
            .and_then(|slot| serde_json::from_value(slot).ok())
            .ok_or(RecurringJobError::InvalidPayload)?;
        let slot = self.interval.slot_at(starts_at)?;
        if slot.starts_at() != starts_at {
            return Err(RecurringJobError::InvalidPayload);
        }
        Ok(slot)
    }

    /// Returns the row for the first slot after both `job`'s slot and `now`.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::slot_of`], and [`RecurringJobError::Interval`]
    /// when the next slot is out of range.
    pub fn next_job(&self, job: &Job, now: Timestamp) -> Result<NewJob, RecurringJobError> {
        let slot = self.slot_of(job)?;
        Ok(self.job_for(self.interval.next_slot(slot, now)?))
    }

    /// Enqueues the slot containing `now`, or the next slot when the current
    /// one already failed. Safe to call from every replica at startup.
    ///
    /// # Errors
    ///
    /// [`StoreError::InvalidInput`] when the slot is out of range, and the
    /// errors of [`JobStore::enqueue`].
    pub async fn enqueue_initial(
        &self,
        store: &dyn JobStore,
        now: Timestamp,
    ) -> Result<EnqueueOutcome, StoreError> {
        let current = self.interval.slot_at(now).map_err(invalid_slot)?;
        let outcome = store.enqueue(self.job_for(current)).await?;
        if outcome.job.status != JobStatus::Failed {
            return Ok(outcome);
        }
        let next = self
            .interval
            .next_slot(current, now)
            .map_err(invalid_slot)?;
        store.enqueue(self.job_for(next)).await
    }
}

fn invalid_slot(error: FixedUtcIntervalError) -> StoreError {
    StoreError::InvalidInput(format!("recurring job slot: {error}"))
}

/// A row that does not describe a slot of its [`RecurringJob`].
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RecurringJobError {
    /// The row is of another type or its payload has no aligned slot.
    #[error("recurring job payload has no aligned slot")]
    InvalidPayload,
    /// The slot or the next slot is out of range.
    #[error(transparent)]
    Interval(#[from] FixedUtcIntervalError),
}
