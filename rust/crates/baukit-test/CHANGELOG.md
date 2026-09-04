# Changelog

All notable changes to `baukit-test` are documented here.

## [Unreleased]

## [0.3.0] - 2026-09-04

### Added

- Add a PostgreSQL live-row cap race check for concurrent last-slot creates,
  updates at capacity, soft-delete release, live counts, and stable limit codes.
- Add PostgreSQL inbox conformance checks for scoped replay, real concurrent
  replay, transaction rollback, owner isolation, and durable outcomes.
- Add HMAC-SHA256 webhook signing helpers and a bounded scripted HTTP receiver
  for retry and idempotency tests.
- Add a scripted credential-probe HTTP server and a provider-neutral
  conformance suite for health mapping, retry hints, timeouts, invalid data,
  and response bounds.

### Changed

- Reuse the production resource-budget measurements from `baukit-core` while
  keeping the existing test-helper names available.
- Let `InMemoryApiTokenStore::fail_with` script typed internal failures and
  policy rejections after the `ApiTokenStore` error contract changed.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

### Added

- Add resource-limit conformance helpers for boundary checks, ingress parity,
  stable reason codes, update-at-capacity behavior, and soft-delete recovery.

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

### Changed

- Pin the PostgreSQL test container to PostgreSQL 18 Alpine.

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-test`.
