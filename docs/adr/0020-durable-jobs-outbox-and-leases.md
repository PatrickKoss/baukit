# ADR 0020: Durable jobs run from a PostgreSQL outbox with attempt-bound leases

## Status
Accepted, 2026-11-27.

## Context
Work that must survive a crash, such as event delivery, erasure steps and provider pulls, needs to commit with the domain write and run at least once. A separate queue cannot join the domain transaction.

A lease by worker id alone was not enough. Two runner slots share one worker id. After a lease expired and a sibling slot reclaimed the job, the stale slot could still complete it.

## Decision
`baukit-jobs` stores jobs in `job_outbox`. Products copy its migrations in order and enqueue in the domain transaction. `PostgresJobStore::claim` picks one row with `FOR UPDATE SKIP LOCKED`, increments `attempts` and sets `locked_by` and `locked_until`. See the [crate README](../../rust/crates/baukit-jobs/README.md).

Every terminal write takes a `JobLease` (`job_id`, `worker_id`, `attempt`), built with `JobLease::of`. The lease predicate requires `status = 'running'`, the worker, a live `locked_until` and `attempts = $attempt`. `complete`, `record_failure`, `cancel`, `complete_lease_in_transaction` and `check_lease_held_in_transaction` all use it. A failed predicate is `LeaseLost`. `request_cancellation` cancels pending jobs and flags running ones, and the runner polls `cancellation_requested`.

## Enforced by
- `JobStore` signatures take `&JobLease`, so the compiler rejects attempt-agnostic calls.
- TLA+ model [M-02 JobLease](../../models/JobLease/README.md): `JobLease.cfg` must pass and `JobLease_current.cfg` must violate `WriteByLatestAttempt`. Gate step `models`.
- `stale_attempt_never_holds_the_lease` in `rust/crates/baukit-jobs/src/lease.rs`, and `stale_attempt_of_the_same_worker_loses_the_lease` and `sql_lease_predicate_agrees_with_the_mirror` in `rust/crates/baukit-jobs/tests/postgres_lease.rs`.
- `succeeded_jobs_were_completed_by_their_latest_attempt` in `rust/crates/baukit-jobs/tests/postgres_state_machine.rs`. Gate step `rust-tests`.

## Consequences
Products get transactional enqueue and at-least-once execution. Handlers must be idempotent. Custom `JobStore` implementations must carry the attempt.

## History
- 2026-11-25, `db073118`: leases bound to the claimed attempt.
- 2026-11-26, `a173747a`: the attempt-agnostic `complete_in_transaction` and `check_lease_in_transaction` removed.
