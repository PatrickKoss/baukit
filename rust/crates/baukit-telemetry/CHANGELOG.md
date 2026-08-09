# Changelog

All notable changes to `baukit-telemetry` are documented here.

## [Unreleased]

### Fixed

- PostgreSQL pool acquisition duration now exports as a Prometheus histogram with standard buckets, matching the shared dashboard's `_bucket` query.

## [0.2.0] - 2026-08-08

### Added

- `OTEL_SDK_DISABLED=true` and `TelemetryBuilder::sdk_disabled` now skip the OpenTelemetry provider/exporter pipeline while retaining logging and Prometheus metrics.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
