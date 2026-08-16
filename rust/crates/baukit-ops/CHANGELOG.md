# Changelog

All notable changes to `baukit-ops` are documented here.

## [Unreleased]

## [0.6.0] - 2026-08-16

### Changed

- Advanced with the coordinated baukit 0.6.0 release train; no crate-specific
  API changes.

## [0.5.1] - 2026-08-14

## [0.5.0] - 2026-08-10

## [0.4.0] - 2026-08-10

## [0.3.5] - 2026-08-09

## [0.3.4] - 2026-08-09

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

## [0.3.0] - 2026-08-09

### Fixed

- PostgreSQL pool metric families, including `db_pool_acquire_timeouts_total`, are registered with zero values before their first event.

## [0.2.0] - 2026-08-08

### Added

- Instrumented PostgreSQL `begin` helper plus prominent guidance for replacing implicit `&PgPool` executors with `acquire` and an explicit connection.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
