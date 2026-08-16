# Changelog

## [Unreleased]

## [0.6.0] - 2026-08-16

### Added

- Provider-directed retry delays and stable permanent/attempt-exhausted reasons
  on terminal `failed` jobs.
- Existing v0.5.1 installations must apply
  `migrations/0002_baukit_jobs_failure_reason.sql`; it adds and backfills
  `failure_reason` before enforcing its value and status consistency checks.

## [0.5.1] - 2026-08-14

## [0.5.0] - 2026-08-10

## [0.4.0] - 2026-08-10

## [0.3.5] - 2026-08-09

## [0.3.4] - 2026-08-09

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

## [0.3.0] - 2026-08-09

- Add a durable PostgreSQL job outbox, worker runner, cancellation, retries,
  readiness, and standard worker telemetry.
