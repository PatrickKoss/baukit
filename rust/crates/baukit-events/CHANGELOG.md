# Changelog

All notable changes to `baukit-events` are documented here.

## [Unreleased]

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

## [0.6.0] - 2026-10-02

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Changed

- Break: `EventEnvelope` serializes camelCase field names while staying schema version 1:
  `event_id`, `user_id`, `occurred_at`, `source_app`, and `schema_version` are now `eventId`,
  `userId`, `occurredAt`, `sourceApp`, and `schemaVersion`. `type` and `payload` are
  unchanged. Payload keys in the shared fixtures are camelCase.
- Break: `IngestOutcome` serializes `ledger_entry_id` as `ledgerEntryId`.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-events`.
