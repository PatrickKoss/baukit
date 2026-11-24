# Changelog

## [Unreleased]

## [0.30.1] - 2026-11-24

## [0.30.0] - 2026-11-23

## [0.29.0] - 2026-11-23

- Require sqlx 0.9.1. sqlx-postgres 0.9.0 counted a transaction only after `BEGIN` returned, so a `begin()` future dropped mid-await left the server connection inside a transaction. The pool then handed that connection to the next borrower, whose statements ran inside it. 0.9.1 counts the transaction before sending `BEGIN`, and products resolve it through this crate's minimum.

## [0.28.8] - 2026-11-22

## [0.28.7] - 2026-11-22

## [0.28.6] - 2026-11-22

## [0.28.5] - 2026-11-22

## [0.28.4] - 2026-11-19

## [0.28.3] - 2026-11-19

## [0.28.2] - 2026-11-18

## [0.28.1] - 2026-11-18

## [0.28.0] - 2026-11-17

- BREAKING: chrono is replaced by jiff 1.0. `DataSubjectConfig::export_retention`, `operation_retention` and `identity_deletion_timeout` are `jiff::SignedDuration`. Operation and export `created_at` and `completed_at` are `jiff::Timestamp`, `retain_until` is `jiff::civil::Date`, and `purge_expired` takes a `jiff::Timestamp`. See the type mapping in `cli/CHANGELOG.md`.

## [0.27.4] - 2026-11-17

## [0.27.3] - 2026-11-17

## [0.27.2] - 2026-11-17

## [0.27.1] - 2026-11-17

## [0.27.0] - 2026-11-17

## [0.26.7] - 2026-11-17

## [0.26.6] - 2026-11-17

## [0.26.5] - 2026-11-17

## [0.26.4] - 2026-11-16

## [0.26.3] - 2026-11-16

## [0.26.2] - 2026-11-16

## [0.26.1] - 2026-11-16

## [0.26.0] - 2026-11-16

## [0.25.2] - 2026-11-13

## [0.25.1] - 2026-11-13

## [0.25.0] - 2026-11-13

## [0.24.0] - 2026-11-12

## [0.23.0] - 2026-11-12

## [0.22.0] - 2026-11-11

## [0.21.0] - 2026-11-10

## [0.20.0] - 2026-11-10

## [0.19.2] - 2026-11-08

## [0.19.1] - 2026-11-08

## [0.19.0] - 2026-11-07

## [0.18.3] - 2026-11-07

## [0.18.2] - 2026-11-06

## [0.18.1] - 2026-11-05

## [0.18.0] - 2026-11-05

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

## [0.15.0] - 2026-11-02

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

## [0.13.0] - 2026-10-31

## [0.12.2] - 2026-10-31

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

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
