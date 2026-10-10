# Changelog

All notable changes to `baukit-jobs` are documented here.

## [Unreleased]

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
