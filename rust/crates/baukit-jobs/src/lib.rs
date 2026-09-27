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
//! [`POSTGRES_MIGRATION_0002_SQL`], and [`POSTGRES_MIGRATION_0003_SQL`] into
//! their own ordered SQLx migrations. The crate never runs migrations at
//! process startup. Use
//! [`PostgresJobStore::enqueue_in_transaction`] when a domain write and its job
//! need to commit atomically. Likewise, a handler which writes PostgreSQL can
//! use [`PostgresJobStore::complete_in_transaction`] as the last statement in
//! the same transaction. After that transaction commits, call
//! [`JobCancellation::mark_completed_in_transaction`] before returning success
//! so the runner does not attempt the transition again.
//!
//! # Runtime composition
//!
//! A runner claims only rows whose `job_type` its handler declares in
//! [`JobHandler::job_types`]. Several runners with disjoint handlers can share
//! one outbox; a type no running handler declares stays pending and keeps the
//! oldest-age gauge rising.
//!
//! [`WorkerRunner::run`] accepts [`baukit_runtime::ShutdownToken`]. It stops
//! claiming after shutdown begins and drains its `JoinSet`; when spawned through
//! [`baukit_runtime::TaskSupervisor`], the process-wide drain deadline remains
//! the single upper bound.

#![deny(missing_docs)]

mod error;
mod metrics;
mod model;
mod recurring;
mod runner;
mod store;

pub use error::{JobError, RunnerError, StoreError};
pub use model::{
    ClaimedJob, EnqueueOutcome, FailureDisposition, Job, JobFailureReason, JobStatus, NewJob,
    TerminalJobCleanupOutcome, TerminalJobCutoffs,
};
pub use recurring::{FixedUtcInterval, FixedUtcIntervalError, FixedUtcSlot};
pub use runner::{JobCancellation, JobFuture, JobHandler, WorkerConfig, WorkerRunner};
pub use store::{JobStore, MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE, PostgresJobStore, StoreFuture};

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
