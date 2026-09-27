# Changelog

All notable changes to `baukit-http` are documented here.

## [Unreleased]

### Added

- `HttpOptions::with_additional_exposed_headers` adds response headers to the CORS exposed set,
  next to `with_additional_allowed_headers`. Invalid names return
  `HttpOptionsError::InvalidHeaderName` and duplicates are ignored.
- `ResponseCachePolicy` and `HttpOptions::with_response_cache_policy` control the default
  `Cache-Control` header. `ResponseCachePolicy::HandlerOwned` turns the default off.

### Changed

- Breaking: the default CORS exposed set now also contains `Retry-After`, `RateLimit-Limit`,
  `RateLimit-Remaining`, and `RateLimit-Reset`, the headers `baukit-ratelimit` emits. Browsers can
  read them on responses that pass through `finalize` or `layers`.
- Breaking: every response from `finalize` or `layers` gets `Cache-Control: private, no-store`
  unless the handler or an inner layer already set `Cache-Control`. This includes error, timeout,
  panic, 404, and preflight responses. Set `ResponseCachePolicy::HandlerOwned` to keep the previous
  behavior.
- The `HttpOptionsError::InvalidHeaderName` message now reads `invalid CORS header name`, because
  it covers exposed headers too.
- `baukit-http` depends on `baukit-core` with the `pagination` feature and no longer depends on
  `base64` or `ring` directly.
- Break: the error envelope field `request_id` is now `requestId`, through the re-exported
  `ErrorBody`. This covers every `ApiError`, extractor rejection, timeout, panic, body-limit,
  404, and 405 response.
- Break: the `RequestLocale` rejection detail key `accept_language` is now `acceptLanguage`.
- Break: `Page` responses carry `nextCursor` instead of `next_cursor`, through `baukit-core`.

### Removed

- Breaking: the `pagination` module and the root re-exports of `Cursor`, `Page`, `PageKey`,
  `PageParams`, `PaginationError`, `DEFAULT_PAGE_LIMIT`, and `MAX_PAGE_LIMIT`. They moved to
  `baukit_core::pagination` with no re-export at the old path. `From<PaginationError> for ApiError`
  stays in this crate.

### Migration

- Add `baukit-core` with `features = ["pagination"]` to every crate that imports the pagination
  types, and replace `baukit_http::{Cursor, Page, ...}` and `baukit_http::pagination::...` with
  `baukit_core::pagination::...`.
- Delete product middleware that overwrites `Access-Control-Expose-Headers` to add `Retry-After`
  or `RateLimit-*`. Pass any other product headers to `with_additional_exposed_headers`.
- Delete product middleware that sets `Cache-Control: private, no-store` or `no-store` on every
  response. Routes that need a different policy set `Cache-Control` in the handler, which wins.
- Apply `finalize` after authentication and rate-limit layers so their rejections carry CORS
  headers.

## [0.4.0] - 2026-09-12

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
