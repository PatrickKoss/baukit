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
- Add `check_purge_horizon_conformance` and `assert_purge_horizon_conformance` with the
  `PurgeHorizonAdapter` trait, `PurgeHorizonPull`, and `PullPause`. The check covers horizon
  monotonicity, batch limits, a purge racing an open pull, cursors at and below the horizon, owner
  isolation, and erasure.
- Re-export `assert_openapi_camel_case` and `check_openapi_camel_case` from `baukit-openapi`.
- Add `check_replay_safe_mutation_conformance` and `assert_replay_safe_mutation_conformance` with
  the `ReplaySafeMutationAdapter` trait, `ReplayRequest`, `ReplayOperation`, `ReplayOutcome`,
  `ReplaySnapshot`, `CommitCheckpoint`, `InjectedRollback`, `ReplayConformanceInputs`, and
  `ReplayConformanceError`. The check covers a lost response, equivalent and changed input,
  simultaneous requests with one key, owner and operation isolation, rollback before commit, a
  crash after commit, expiry, bounded cleanup, and erasure.
- Add `assert_response_matches_openapi` and `check_response_matches_openapi` with
  `ObservedResponse` and `OpenApiResponseError`. They check a real response's status, media type,
  and JSON body against its operation in a serialized OpenAPI 3.1 document, using JSON Schema
  2020-12 with format validation. The crate now depends on `jsonschema` 0.58.1 without default
  features.

### Changed

- `InMemoryApiTokenStore` stores `ApiTokenRecord::grants` on the token and
  keeps the latest `last_used_at` when touches arrive out of order.

- Break: `PostgresTestError` gains `InvalidAppRole`, `InvalidConnectionUrl`,
  and `Setup` variants. Exhaustive matches on it must handle them.
- Break: `check_auth_router_conformance` now expects the
  `error_description="expired"` and `error_description="invalid"` bearer
  challenges that `baukit-auth` sends.
- With `sqlx-postgres`, the container fixture now waits until PostgreSQL
  accepts a connection before it returns.
- Break: `check_auth_router_conformance` expects `requestId` in the unauthenticated error
  envelope instead of `request_id`.

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
