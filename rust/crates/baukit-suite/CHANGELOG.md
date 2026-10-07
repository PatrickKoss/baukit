# Changelog

## [Unreleased]

- Add product-neutral peer linking, PKCE, sealed per-link secrets and catalog-driven payload validation.
- Add transactional outbox and inbox stores, signed delivery and revocation, rate limits, circuit accounting, replay, HTTP routes, job handling and hourly cleanup.
- Send bounded peer revokes before account erasure, then delete suite rows in the product transaction.
- Lock the product owner before suite cleanup so concurrent domain writes cannot leave delivery jobs behind.
- Serialize replay, connection-test and disconnect jobs with erasure before locking links.
- Add a domain transaction owner lock and reject unfenced enqueues during erasure without aborting the product transaction.
- Validate shared protocol fixtures and payload vectors against unit tests and PostgreSQL router tests.
