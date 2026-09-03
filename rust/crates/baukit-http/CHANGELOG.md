# Changelog

All notable changes to `baukit-http` are documented here.

## [Unreleased]

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
