# Changelog

All notable changes to `baukit-http` are documented here.

## [Unreleased]

### Added

- Standard `unauthenticated` and `permission_denied` error constructors for authentication and authorization boundaries.
- Additive CORS request-header configuration through `HttpOptions::with_additional_allowed_headers`.
- Configurable `ApiJson` rejection codes through `HttpOptions::with_json_rejection_code`.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
