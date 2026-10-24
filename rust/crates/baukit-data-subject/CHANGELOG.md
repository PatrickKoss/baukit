# Changelog

## [Unreleased]

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Apply job migration 0004 in PostgreSQL fixtures for the store's new key columns.

## [0.10.32] - 2026-10-21

- Run the shared service conformance check against the PostgreSQL fixture
  product with erasure only and with exports, using real authentication and jobs.

- Add opt-in exports through `DataSubjectExportProduct`, using the existing
  acceptance, operation store and lease-aware worker.
- Store bounded JSON parts and their signed digests atomically. Reject oversized
  records with `data_subject_export_record_too_large` and no receipt.
- Add scope-isolated export status and download routes, seven-day default part
  retention and bounded part purge independent of operation retention.
- Add the exports migration with completed-export ownership constraints and
  exact byte and digest checks. Products manage table access through their own grants.

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
