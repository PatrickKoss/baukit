# Changelog

## [Unreleased]

- Add the default `server` feature with PostgreSQL operations, authenticated
  erasure routes, public discovery, transactional jobs and signed results.
- Resolve lookup identifiers during acceptance and queue only internal subject
  IDs. Apply profile-erasure fences and identity retention during execution.
- Add bounded operation purge and safe terminal job-failure reconciliation.
- Wait for configured identity deletion before signing a completed receipt;
  provider failures leave the data subject operation failed without a receipt.
- Back off identity checks to 60 seconds and fail without a receipt when the
  configured wait expires, by default after 24 hours. Make discovery publicly
  cacheable for five minutes.

- Add data subject v1 contract types, request validation and normalization,
  Ed25519 DSSE receipt signing and verification, and independent receipt fixtures.
