# Changelog

All notable changes to `baukit-jobs` are documented here.

## [Unreleased]

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

- Fix `WorkerRunner::run` reusing one timestamp for every claim in a poll loop. A job enqueued during the loop could be claimed with `updated_at` before its `created_at`, which violates the outbox check. Each claim now takes a fresh time.

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Add `PostgresJobStore::enqueue_deduplicated` and
  `enqueue_deduplicated_in_transaction` to share a key only while a job is
  pending or running. Existing idempotency behavior stays unchanged.
- Add `enqueue_pending_deduplicated` and
  `enqueue_pending_deduplicated_in_transaction` for one pending follow-up while
  another job runs. Claiming clears the pending key so retries cannot collide
  with a follow-up. The two deduplication modes use independent keys.
- Breaking: add public `Job::dedupe_key` and `Job::pending_dedupe_key` fields.
  Custom stores and tests that construct `Job` must set them, usually to `None`.
  Keep `Job` constructible for custom `JobStore` implementations. Enqueue and
  claim now require migration 0004, including calls to existing methods,
  because returned jobs include both columns.
- Add `migrations/0004_baukit_jobs_active_deduplication.sql` and
  `POSTGRES_MIGRATION_0004_SQL`. Products must copy it as a new ordered
  migration before upgrading the store. Worker and authenticated
  backend templates include it as migration 0007.
- Document migration 0004 deployment order in the upgrade skill and backfill
  only the newest legacy pending row per key in the adoption example.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

- Keep polling a job's handler and timeout while its cancellation query waits
  for a database connection.

## [0.10.18] - 2026-10-11

- Drop the handler future before recording timeout or cancellation outcomes.
  This releases handler-owned transactions so their job row locks cannot
  deadlock the runner's update.

## [0.10.17] - 2026-10-11

- Add `FixedUtcJitter` and `FixedUtcInterval::run_at` for stable per-key,
  whole-second offsets that stay inside their UTC slot, including at fraction
  `1.0`. Slot identifiers and progression stay unchanged.
- Add transaction cancellation by exact idempotency key or literal key prefix,
  scoped to a job type, with pending and running counts.
- Add `PostgresJobStore::check_lease_in_transaction` to check lease ownership
  without completion and hold the row lock through the caller's transaction.
  It shares the completion predicate. No schema migration is needed.

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

- Add `PostgresJobStore::retain_kinds` to keep succeeded, cancelled and failed
  jobs out of generic cleanup when another component owns their retention.
  It can be combined with `retain_failed_kinds` for failures that need repair.

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Accept `&mut PgConnection` in `enqueue_in_transaction` so callers can share their domain transaction. Existing `Transaction` callers use deref coercion.

## [0.9.0] - 2026-10-07

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

- Finish claim, enqueue, and readiness transactions in owned tasks when callers cancel. Abandoned claims recover after lease expiry.
- Allow terminal cleanup to retain failed job kinds for repair. Generated OIDC workers retain identity deletion failures.

- Use `job_kind` for worker metrics so Prometheus keeps the handler type separate from the scrape job. Update worker dashboards, recording rules, and alerts.

- Remove the extra blank line at the end of the published jobs migration so copied migrations pass git diff --check.

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

### Fixed

- Select aws-lc-rs for sqlx TLS so rustls has one crypto provider. Products
  must use sqlx `tls-rustls-aws-lc-rs` instead of `tls-rustls`, including dev
  dependencies, and disable the `ring` default in testcontainers and
  testcontainers-modules.

## [0.6.0] - 2026-10-02

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Changed

- **Breaking:** `JobStore::claim` takes a `job_types: &[&str]` argument after
  `worker_id` and claims only pending rows or expired leases whose `job_type` is
  in that set. An empty set, or a blank or overlong entry, returns
  `StoreError::InvalidInput`. Custom stores must add the parameter and apply the
  filter; direct callers of `PostgresJobStore::claim` must pass the types they
  handle.
- `WorkerRunner` passes its handler's `job_types()` to every claim. Runners with
  disjoint handlers can share one outbox. A type that no running handler
  declares stays pending and shows in `worker_queue_oldest_age_seconds`.
- `JobStore::ready` on `PostgresJobStore` probes the filtered claim shape.

### Added

- Added `migrations/0003_baukit_jobs_claim_by_type.sql` and
  `POSTGRES_MIGRATION_0003_SQL`, which rebuild `job_outbox_claim_idx` on
  `(job_type, run_after, created_at, id)` for pending rows.

### Migration

- Add the 0003 migration after the existing baukit-jobs migrations. Without it,
  claims stay correct but scan pending rows of other types.
- A worker whose handler did not declare every type it expected to run now
  leaves the undeclared types pending. Add them to `job_types()` or start a
  runner whose handler declares them.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

### Added

- Added bounded `PostgresJobStore::cleanup_terminal_jobs` deletion with separate
  succeeded, cancelled, and failed cutoffs and committed per-status counts.
- Added fixed whole-second UTC slot calculation and canonical idempotency
  identifiers for self-enqueuing recurring jobs.

### Migration

- No schema migration is needed. Terminal cleanup is a concrete PostgreSQL
  store method, so existing `JobStore` implementations remain compatible.
- Existing recurring jobs can replace local UTC rounding and slot-key code.
  Pass the current payload slot and the handler clock to `next_slot`, use the
  returned boundary as `run_after`, and use its identifier as the idempotency
  key. Keep interval and catch-up choices in the application.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-jobs`.
