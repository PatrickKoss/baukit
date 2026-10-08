# Changelog

## [Unreleased]

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
