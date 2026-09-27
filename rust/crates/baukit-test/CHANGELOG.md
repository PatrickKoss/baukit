# Changelog

All notable changes to `baukit-test` are documented here.

## [Unreleased]

### Added

- Add `PostgresTestOptions` to start the PostgreSQL fixture with another image
  and tag, a login app role without superuser or `BYPASSRLS`, and migrations.
  `start_postgres` and `start_postgres_with_migrations` keep their signatures
  and now delegate to it.
- Add `PostgresTestContainer::create_database` and `PostgresTestDatabases` for
  one database per test on a shared container or on an external server such as
  `DATABASE_URL`. Each `PostgresTestDatabase` drops its database when it drops.
- Add `MockOidcServer::jwks_url` for verifiers built with `from_jwks_uri`, and
  `MockOidcServer::mint_with_key_id` to sign with the active key under an
  unpublished `kid`. Document the existing `jwks_request_count` and
  `set_jwks_delay` for JWKS cache tests.

### Changed

- Break: `PostgresTestError` gains `InvalidAppRole`, `InvalidConnectionUrl`,
  and `Setup` variants. Exhaustive matches on it must handle them.
- Break: `check_auth_router_conformance` now expects the
  `error_description="expired"` and `error_description="invalid"` bearer
  challenges that `baukit-auth` sends.
- With `sqlx-postgres`, the container fixture now waits until PostgreSQL
  accepts a connection before it returns.

## [0.4.0] - 2026-09-12

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
