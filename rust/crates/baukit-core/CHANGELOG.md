# Changelog

All notable changes to `baukit-core` are documented here.

## [Unreleased]

### Added

- Add the optional `pagination` feature and the `pagination` module with `Cursor`, `Page`,
  `PageKey`, `PageParams`, `PaginationError`, `DEFAULT_PAGE_LIMIT`, and `MAX_PAGE_LIMIT`, moved
  from `baukit-http` so domain crates can use them without Axum. The feature adds `base64`,
  `ring`, and `uuid`; the default build keeps its three dependencies.
- Add `pagination::MAX_CURSOR_BYTES` (4096). Breaking: `Cursor::decode` rejects longer input with
  `PaginationError::InvalidCursor` before decoding, and `Cursor::encode` returns the same error
  instead of issuing a longer cursor.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

### Added

- Add production measurements and checks for trimmed Unicode scalars, compact
  JSON UTF-8 bytes, byte slices, and collection slices.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-core`.
