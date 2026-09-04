# Changelog

All notable changes to `baukit-auth` are documented here.

## [Unreleased]

### Added

- Add validated `ApiTokenPolicyRejection` codes with at most eight numeric
  details so adapters can return safe policy decisions.

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
