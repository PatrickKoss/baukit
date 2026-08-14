# Changelog

All notable changes to `baukit-telemetry` are documented here.

## [Unreleased]

## [0.5.1] - 2026-08-14

## [0.5.0] - 2026-08-10

## [0.4.0] - 2026-08-10

## [0.3.5] - 2026-08-09

## [0.3.4] - 2026-08-09

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

### Changed

- The `testing` environment uses deployed JSON logging and OTLP endpoint policy.

## [0.3.0] - 2026-08-09

### Fixed

- PostgreSQL pool acquisition duration now exports as a Prometheus histogram with standard buckets, matching the shared dashboard's `_bucket` query.

## [0.2.0] - 2026-08-08

### Added

- `OTEL_SDK_DISABLED=true` and `TelemetryBuilder::sdk_disabled` now skip the OpenTelemetry provider/exporter pipeline while retaining logging and Prometheus metrics.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
