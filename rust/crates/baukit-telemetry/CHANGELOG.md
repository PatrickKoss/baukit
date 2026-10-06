# Changelog

All notable changes to `baukit-telemetry` are documented here.

## [Unreleased]

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

## [0.6.0] - 2026-10-02

### Changed

- Breaking: moved to OpenTelemetry 0.33 and tracing-opentelemetry 0.34. The re-exported
  `opentelemetry` crate and `OpenTelemetrySpanExt` now come from those versions, so products
  that name OpenTelemetry types must match them. The OTLP exporter now retries a failed export
  up to three times with exponential backoff, which is the upstream default.

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-telemetry`.
