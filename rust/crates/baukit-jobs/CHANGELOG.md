# Changelog

All notable changes to `baukit-jobs` are documented here.

## [Unreleased]

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
