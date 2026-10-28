# Changelog

## [Unreleased]

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Clarify that handlers overriding `prepare_at` must delegate `prepare` to it.

- Keep entitlement events retryable when a custom inbox skips preparation. Pass the receiver clock to `PlatformEventHandler::prepare_at` for deterministic free-cache deadlines.
- Document that erasure removes revision cursors and products must filter late events for erased subjects.

- Add per-peer hosted `authorize_url` overrides with config and registry validation. The full path and suite protocol query parameters are preserved.
- Breaking: `PeerUrls`, `PeerRegistrySettings` and `SuiteConfig` are non-exhaustive. Replace external struct literals with `PeerUrls::new`, `PeerRegistrySettings::default` and their builders, or mutate defaults. `ActivePeer` gains `authorize_url: Option<url::Url>`, parsed during registry validation. This requires a minor release.
- Add the read-only `SuiteLinksRead` service port, implemented by `SuiteLinkService`.
- Apply job migration 0004 in PostgreSQL fixtures for the store's new key columns.
- Add read-through entitlement refresh with per-subject single-flight, explicit free fallback, transactional cache erasure and batched receipt retention. Configure the hub source and event prefix, prepare pulls before receipt transactions, and keep cached ticket verification available during JWKS refresh.
- Revise the unreleased platform API: public event context constructors, `PlatformEventDecoder`, `PlatformEventHandler::prepare`, cache revision and erasure methods, a required source for hub reads, `EntitlementSourceError`, and detailed configuration errors from `ticket_verifier`.
- Add the optional `platform` feature with typed hub events, a signed receiver, transactional PostgreSQL receipts, entitlement caches, standalone reads and a cached Ed25519 ticket verifier.

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

- Check erasure admission before notifying peers. Old-key replays after re-onboarding, fenced sessions, invalid keys and foreign keys leave the current profile's suite links intact.

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

- Export the sorted suite error codes from the same definition as the Rust mappings and OpenAPI enum.
- The OpenAPI `lastFailureCode` enum on suite link delivery status is now `SuiteErrorCode` with every value in `SUITE_ERROR_CODES`, replacing the ten-value `SuiteDeliveryFailureCode`. Generated clients with an exhaustive map over it need entries for the added codes.

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

- Export `SUITE_JOB_TYPES` for products to pass to `PostgresJobStore::retain_kinds`.
  This keeps suite delivery and revoke history until suite cleanup removes it
  after 30 days, even when generic job cleanup uses a shorter cutoff.

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

- Breaking: Add `SuiteStoreError::QuotaExceeded { retry_after_seconds }` for applier quotas that reset. Exhaustive matches need a new arm. Inbound returns 429 `suite_quota_exceeded` with `Retry-After` of at least one second. Permanent owner caps keep returning `Capped`.

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

- Keep hourly cleanup alive after storage errors. Log each failure and count it in `suite_cleanup_failures_total`.
- Guard cleanup across replicas and lock affected owners before links and jobs. Add `003_suite_lock_order.sql` to record failed jobs without locking links, then account for failures before health reads and writes.
- Add a default `SuiteEventApplier::lock_owner` hook before ingest locks a link.
- Return 422 `suite_payload_invalid` for typed payload rejections. Export the field validators and keep existing applier signatures.
- Accept empty boolean settings as false.
- Cover HTTP error codes, rate-limit keys, delivery revocation, link isolation, failed exchanges, replay, URLs and wire contracts in protocol tests.
- Use real catalog types in the event-id vectors. Products that copy the vectors should refresh them.

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

- Keep the default build free of database, Tokio and HTTP dependencies. Gate SQLx ports and stores behind `postgres`, services behind `runtime`, and the guarded peer client behind `delivery`.
- Check each feature build with Clippy and test a domain consumer without features.

## [0.10.0] - 2026-10-08

- Look up metadata for the own app and inactive peers by id. Share peer validation vectors with the client.
- Document how existing suite migrations and pending release packages adopt the crate.
- Add product-neutral peer linking, PKCE, sealed per-link secrets and catalog-driven payload validation.
- Add transactional outbox and inbox stores, signed delivery and revocation, rate limits, circuit accounting, replay, HTTP routes, job handling and hourly cleanup.
- Send bounded peer revokes before account erasure, then delete suite rows in the product transaction.
- Lock the product owner before suite cleanup so concurrent domain writes cannot leave delivery jobs behind.
- Serialize replay, connection-test and disconnect jobs with erasure before locking links.
- Add a domain transaction owner lock and reject unfenced enqueues during erasure without aborting the product transaction.
- Validate shared protocol fixtures and payload vectors against unit tests and PostgreSQL router tests.
