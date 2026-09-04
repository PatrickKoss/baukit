# Changelog

All notable changes to `baukit-auth` are documented here.

## [Unreleased]

## [0.3.0] - 2026-09-04

### Added

- Add validated `ApiTokenPolicyRejection` codes with at most eight numeric
  details so adapters can return safe policy decisions.
- Add `establish_principal` for Axum compositions that need a verified
  `Principal` before rate limiting or other request middleware. Missing
  credentials continue without a principal; presented invalid credentials use
  the existing authentication envelope.

### Changed

- Change every `ApiTokenStore` operation from `String` failures to
  `ApiTokenStoreError`. Internal adapter diagnostics now map to the generic
  `ApiTokenError::Storage`; policy rejections map to
  `ApiTokenError::PolicyRejected`.

### Migration

- Update each product adapter to return `ApiTokenStoreError`. Wrap SQL and
  provider errors with `ApiTokenStoreError::internal`. Replace encoded policy
  strings with `ApiTokenPolicyRejection` and update API mappings to inspect its
  code and numeric details.
- Replace product-owned principal-caching middleware with
  `middleware::from_fn_with_state(auth, establish_principal)`. Protected route
  extractors remain unchanged and reuse the cached principal.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

### Fixed

- Keep API token validation compatible with current stable Clippy's
  `nonminimal_bool` lint.

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-auth`.
