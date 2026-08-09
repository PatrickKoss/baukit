# Changelog

All notable changes to `baukit-http` are documented here.

## [Unreleased]

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

## [0.3.0] - 2026-08-09

### Added

- Utoipa-recognized `Query` and `Path` names for the standard extractors; the existing `ApiQuery` and `ApiPath` names remain aliases.
- Repeated query parameters deserialize into collection fields, and `ApiError::validation_field(s)` builds field-level validation details.
- Typed response-envelope and RFC 3339 date-time wire types are re-exported from `baukit-openapi`.

## [0.2.0] - 2026-08-08

### Added

- Standard `unauthenticated` and `permission_denied` error constructors for authentication and authorization boundaries.
- Additive CORS request-header configuration through `HttpOptions::with_additional_allowed_headers`.
- Configurable `ApiJson` rejection codes through `HttpOptions::with_json_rejection_code`.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
