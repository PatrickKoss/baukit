# Changelog

All notable changes to `baukit-http` are documented here.

## [Unreleased]

## [0.3.0] - 2026-09-04

### Added

- `RequestLocale` selects from a product-owned locale set using a percent-decoded query override or
  quality-weighted `Accept-Language`. Configuration fixes the fallback and query rule. Malformed,
  duplicate override, unsupported explicit, and oversized inputs return stable validation errors.
- `JsonRejectionCodes` and `HttpOptions::with_json_rejection_codes` preserve JSON rejection classes.
  Oversized bodies return 413, missing or invalid content types return 415, malformed JSON returns
  400, and data-shape errors return 422. Responses contain fixed safe text without submitted body or
  parser details.

### Compatibility

- Request locale extraction is additive. Existing handlers retain their current behavior until they
  put `RequestLocaleConfig` in Axum state and use the extractor.
- `HttpOptions::default()` and `with_json_rejection_code` retain the previous single-code 400
  response for `ApiJson<T>` rejections during this release cycle. See the README migration section
  before opting into class-specific responses.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

### Fixed

- `ApiError` stores response headers behind a lazily allocated box so the type stays at 104 bytes and
  does not trigger `clippy::result_large_err` in consumers that return `Result<_, ApiError>`.

## [0.1.1] - 2026-09-01

### Added

- `ApiError::with_header` and `ApiError::with_retry_after` add response headers while preserving the
  standard error envelope.

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-http`.
