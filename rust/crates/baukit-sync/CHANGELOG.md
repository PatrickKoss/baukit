# Changelog

All notable changes to `baukit-sync` are documented here.

## [Unreleased]

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

## [0.9.0] - 2026-10-07

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

### Fixed

- Select aws-lc-rs for sqlx TLS so rustls has one crypto provider. Products
  must use sqlx `tls-rustls-aws-lc-rs` instead of `tls-rustls`, including dev
  dependencies, and disable the `ring` default in testcontainers and
  testcontainers-modules.

## [0.6.0] - 2026-10-02

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Added

- Add the `sqlx-postgres` feature. It carries every SQLx helper, so a crate that only needs the
  hybrid logical clock no longer compiles SQLx, `chrono`, or `uuid`.
- Add `horizon::check_pull_cursor`, `PullCursorError`, and the `resync_required` wire constants
  (`RESYNC_REQUIRED_STATUS`, `RESYNC_REQUIRED_CODE`, `HORIZON_REVISION_DETAIL`,
  `FULL_RESYNC_CURSOR`). These need no feature. `HORIZON_REVISION_DETAIL` is the camelCase
  `horizonRevision`, so the stale-cursor detail is `details.horizonRevision` on the wire.
- Add `purge::TombstoneTable`, `purge_tombstone_batch`, `purge_tombstones`, `purge_horizon`, and
  `guard_pull_cursor`. A batch deletes at most the caller's limit of expired tombstones and raises
  each affected owner's horizon in the same transaction, never lowering it. It skips owners whose
  revision counter row is locked instead of waiting. The pull guard locks the counter row with
  `FOR KEY SHARE`, so a purge for that owner cannot commit while the pull transaction is open.
- Add the reference migration `migrations/0002_baukit_sync_purge_horizons.sql` and
  `POSTGRES_PURGE_HORIZONS_MIGRATION_SQL` for the `sync_purge_horizons` table.

### Breaking

- `next_revision`, `ensure_owner`, `current_revision`, and `current_revision_for_update` now need
  `features = ["sqlx-postgres"]`. `chrono`, `sqlx`, and `uuid` are optional dependencies.
- The purge helpers need `sync_purge_horizons`. Its foreign key references `sync_revisions`, so a
  migration that drops `sync_revisions` must drop `sync_purge_horizons` first.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

### Added

- Added a cross-runtime hybrid logical clock with JavaScript-safe encoding, state restoration,
  remote observation, logical rollover, and shared Rust and TypeScript fixtures.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-sync`.
