# Changelog

All notable changes to `baukit-test` are documented here.

## [Unreleased]

## [0.3.4] - 2026-08-09

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

## [0.3.0] - 2026-08-09

### Changed

- Worker metrics conformance now requires `worker_queue_oldest_age_seconds{queue=...}` in addition to the job counter and duration histogram.

## [0.2.0] - 2026-08-08

### Added

- Mock OIDC discovery/JWKS server, rotating RS256 token fixture, and authentication-envelope conformance assertions.
- Opt-in worker metric-family checks for the telemetry specification's job counter and duration histogram.

### Changed

- JWT fixtures use `ring` directly and no longer select a process-global `jsonwebtoken` crypto provider for consumers.
- HTTP test clients resolve through the workspace `reqwest` pin, avoiding a second version in consumer dependency trees.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
