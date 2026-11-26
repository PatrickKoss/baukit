//! Durable PostgreSQL jobs and a product-neutral worker runner.
//!
//! The crate separates a small [`JobStore`] port from its [`PostgresJobStore`]
//! adapter and from [`WorkerRunner`]. Products define static job identifiers,
//! JSON payload contracts, and a [`JobHandler`]; Baukit owns leases, retry and
//! cancellation state transitions, concurrency, shutdown, and platform metrics.
//!
//! # Outbox composition
//!
//! Products must copy [`POSTGRES_MIGRATION_SQL`],
//! [`POSTGRES_MIGRATION_0002_SQL`], [`POSTGRES_MIGRATION_0003_SQL`],
//! [`POSTGRES_MIGRATION_0004_SQL`], [`POSTGRES_MIGRATION_0005_SQL`], and
//! [`POSTGRES_MIGRATION_0006_SQL`] into their own ordered SQLx migrations.
//! The crate never runs migrations at process startup. Use
//! [`PostgresJobStore::enqueue_in_transaction`] when a domain write and its
//! job need to commit atomically. Likewise, a handler which writes PostgreSQL can
//! use [`PostgresJobStore::complete_lease_in_transaction`] as the last statement in
//! the same transaction. After that transaction commits, call
//! [`JobCancellation::mark_completed_in_transaction`] before returning success
//! so the runner does not attempt the transition again.
//!
//! # Runtime composition
//!
//! A runner claims only rows whose `job_type` its handler declares in
//! [`JobHandler::job_types`]. Several runners with disjoint handlers can share
//! one outbox; a type no running handler declares stays pending and no runner
//! reports it in its queue gauges.
//!
//! [`WorkerRunner::run`] accepts [`baukit_runtime::ShutdownToken`]. It stops
//! claiming after shutdown begins, cancels every running attempt's
//! [`JobCancellation`], waits for attempts until
//! [`WorkerConfig::release_reserve`] before the drain deadline, and then
//! releases the attempts still running back to pending without consuming an
//! attempt. Running attempts renew their lease every third of
//! [`WorkerConfig::lease_duration`].
//!
//! Store errors inside the loop are logged and retried with capped, jittered
//! backoff. [`WorkerRunner::store_health`] exposes the result for readiness.
//!
//! # Outbound calls
//!
//! With the `egress` feature, `egress_job_error` turns a failed
//! `baukit_egress::GuardedClient` call into a [`JobError`]: an open circuit or a
//! full bulkhead defers the job without consuming an attempt.

#![deny(missing_docs)]

#[cfg(feature = "egress")]
mod egress;
mod error;
mod lease;
mod metrics;
mod model;
mod recurring;
mod recurring_job;
mod runner;
mod store;
mod wakeup;

#[cfg(feature = "egress")]
pub use egress::egress_job_error;
pub use error::{JobError, RunnerError, StoreError};
pub use lease::{JobLease, LeaseRow, lease_predicate_holds};
pub use model::{
    CancellationOutcome, ClaimedJob, EnqueueOutcome, FailureDisposition, Job, JobFailureReason,
    JobStatus, NewJob, QueueDepth, ReleaseReason, TerminalJobCleanupOutcome, TerminalJobCutoffs,
};
pub use recurring::{FixedUtcInterval, FixedUtcIntervalError, FixedUtcJitter, FixedUtcSlot};
pub use recurring_job::{RecurringJob, RecurringJobError};
pub use runner::{JobCancellation, JobFuture, JobHandler, StoreHealth, WorkerConfig, WorkerRunner};
#[doc(hidden)]
pub use store::CLAIM_CANDIDATE_SQL;
pub use store::{
    JOB_ENQUEUED_CHANNEL, JobStore, MAX_REQUEUE_BATCH_SIZE, MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE,
    PostgresJobStore, StoreFuture, spawn_enqueue_listener,
};
pub use wakeup::JobWakeup;

/// Reference PostgreSQL schema for product-owned migrations.
///
/// Copy this SQL into a product migration; do not execute it dynamically during
/// application startup.
pub const POSTGRES_MIGRATION_SQL: &str = include_str!("../migrations/0001_baukit_jobs.sql");

/// Upgrade from the v0.5.1 PostgreSQL schema to stable failure reasons.
///
/// Apply this SQL as a new product-owned migration after
/// [`POSTGRES_MIGRATION_SQL`]. It backfills existing terminal jobs before
/// enforcing the current failure-reason constraints.
pub const POSTGRES_MIGRATION_0002_SQL: &str =
    include_str!("../migrations/0002_baukit_jobs_failure_reason.sql");

/// Upgrade that leads the pending claim index with `job_type`.
///
/// Apply this SQL as a new product-owned migration after
/// [`POSTGRES_MIGRATION_0002_SQL`]. Claims filter by job type, and this index
/// keeps a claim from scanning pending rows of types its handler does not
/// declare.
pub const POSTGRES_MIGRATION_0003_SQL: &str =
    include_str!("../migrations/0003_baukit_jobs_claim_by_type.sql");

/// Adds independent keys for active-job and pending-only deduplication.
///
/// Apply this SQL as a new product-owned migration after
/// [`POSTGRES_MIGRATION_0003_SQL`] before using any store operation with this
/// version. Existing rows keep null dedupe keys
/// and their permanent idempotency semantics. Products may backfill their own
/// keys before switching producers to either deduplicated enqueue method.
pub const POSTGRES_MIGRATION_0004_SQL: &str =
    include_str!("../migrations/0004_baukit_jobs_active_deduplication.sql");

/// Adds the `deferrals` counter used by [`JobError::deferred`] and shutdown release.
///
/// Apply this SQL as a new product-owned migration after
/// [`POSTGRES_MIGRATION_0004_SQL`] before running this version. The store reads
/// and writes the column on every claim and release.
pub const POSTGRES_MIGRATION_0005_SQL: &str =
    include_str!("../migrations/0005_baukit_jobs_deferrals.sql");

/// Adds `concurrency_key` and the running-key index used by [`WorkerConfig::max_per_key`].
///
/// Apply this SQL as a new product-owned migration after
/// [`POSTGRES_MIGRATION_0005_SQL`] before running this version. Every enqueue
/// and claim reads or writes the column.
pub const POSTGRES_MIGRATION_0006_SQL: &str =
    include_str!("../migrations/0006_baukit_jobs_concurrency_key.sql");
