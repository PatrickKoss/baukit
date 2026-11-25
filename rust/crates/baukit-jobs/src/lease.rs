use jiff::Timestamp;
use uuid::Uuid;

use crate::{ClaimedJob, JobStatus};

/// Ownership of one claimed attempt.
///
/// Every runner slot of a process shares one `worker_id`, so the attempt
/// number is what tells a stale attempt apart from the sibling slot that
/// reclaimed the job after its lease expired.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobLease {
    /// The claimed job.
    pub job_id: Uuid,
    /// Worker identifier written into `locked_by` at claim.
    pub worker_id: String,
    /// Attempt number the claim produced, equal to the row's `attempts`.
    pub attempt: u32,
}

impl JobLease {
    /// Creates a lease for one attempt.
    pub fn new(job_id: Uuid, worker_id: impl Into<String>, attempt: u32) -> Self {
        Self {
            job_id,
            worker_id: worker_id.into(),
            attempt,
        }
    }

    /// Returns the lease a claim by `worker_id` produced for `job`.
    pub fn of(job: &ClaimedJob, worker_id: impl Into<String>) -> Self {
        Self::new(job.id, worker_id, job.attempts)
    }
}

/// The lease columns of one `job_outbox` row.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LeaseRow<'a> {
    /// Current lifecycle state.
    pub status: JobStatus,
    /// `locked_by` column.
    pub locked_by: Option<&'a str>,
    /// `attempts` column.
    pub attempts: u32,
    /// `locked_until` column.
    pub locked_until: Option<Timestamp>,
    /// Whether `cancel_requested_at` is set.
    pub cancel_requested: bool,
}

/// Executable mirror of the SQL predicate every lease-owned write uses.
///
/// `allow_cancellation` is true only for the cancel acknowledgement, which
/// must still match after a cancellation request.
#[must_use]
pub fn lease_predicate_holds(
    row: &LeaseRow<'_>,
    worker_id: &str,
    attempt: u32,
    now: Timestamp,
    allow_cancellation: bool,
) -> bool {
    row.status == JobStatus::Running
        && row.locked_by == Some(worker_id)
        && row.attempts == attempt
        && row.locked_until.is_some_and(|until| until > now)
        && (allow_cancellation || !row.cancel_requested)
}

#[cfg(kani)]
mod proofs {
    use super::*;

    /// K-17: a stale attempt never satisfies the lease predicate.
    ///
    /// Bounds: two worker ids, any `u32` attempts, timestamps from any `i32`
    /// second offset. No loops, so no unwind bound is needed.
    #[kani::proof]
    fn stale_attempt_never_holds_the_lease() {
        let workers = ["worker-a", "worker-b"];
        let row_worker: bool = kani::any();
        let caller_worker: bool = kani::any();
        let row_attempts: u32 = kani::any();
        let stale_attempt: u32 = kani::any();
        kani::assume(stale_attempt != row_attempts);
        let until: i32 = kani::any();
        let now: i32 = kani::any();
        let (Ok(until), Ok(now)) = (
            Timestamp::from_second(i64::from(until)),
            Timestamp::from_second(i64::from(now)),
        ) else {
            return;
        };
        let row = LeaseRow {
            status: JobStatus::Running,
            locked_by: Some(workers[usize::from(row_worker)]),
            attempts: row_attempts,
            locked_until: Some(until),
            cancel_requested: kani::any(),
        };
        assert!(!lease_predicate_holds(
            &row,
            workers[usize::from(caller_worker)],
            stale_attempt,
            now,
            kani::any(),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn running(attempts: u32, until: Timestamp) -> LeaseRow<'static> {
        LeaseRow {
            status: JobStatus::Running,
            locked_by: Some("worker"),
            attempts,
            locked_until: Some(until),
            cancel_requested: false,
        }
    }

    #[test]
    fn only_the_latest_attempt_of_the_owner_holds_an_unexpired_lease() {
        let now = Timestamp::UNIX_EPOCH;
        let later = Timestamp::from_second(1).expect("valid timestamp");
        let row = running(2, later);
        assert!(lease_predicate_holds(&row, "worker", 2, now, false));
        assert!(!lease_predicate_holds(&row, "worker", 1, now, false));
        assert!(!lease_predicate_holds(&row, "other", 2, now, false));
        assert!(!lease_predicate_holds(&row, "worker", 2, later, false));
        let cancelled = LeaseRow {
            cancel_requested: true,
            ..row
        };
        assert!(!lease_predicate_holds(&cancelled, "worker", 2, now, false));
        assert!(lease_predicate_holds(&cancelled, "worker", 2, now, true));
    }
}
