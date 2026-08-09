# Changelog

All notable changes to `baukit-config` are documented here.

## [Unreleased]

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

### Added

- Configuration bootstrap accepts `testing` as a deployed environment.

## [0.3.0] - 2026-08-09

### Added

- Collection-valued configuration fields accept JSON arrays from environment variables (CORS automatically, product fields via `environment_collection`) while all other source strings remain unchanged.

## [0.2.0] - 2026-08-08

### Fixed

- Environment values retain their literal string representation for secrets, including leading-zero and exponent-shaped values.
- The workspace uses a normal caret requirement for `thiserror` so downstream lockfiles are not forced to the exact patch release.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
