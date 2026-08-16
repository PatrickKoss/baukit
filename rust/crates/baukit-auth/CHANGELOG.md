# Changelog

All notable changes to `baukit-auth` are documented here.

## [Unreleased]

## [0.6.1] - 2026-08-16

### Added

- Added an allowlisted `MultiIssuerVerifier` and preserved the verified issuer
  on OIDC principals so applications can safely key identities by
  `(issuer, subject)` across providers.
- Added explicit JWKS endpoint constructors for deployments where public token
  issuers and private key-retrieval addresses differ.

## [0.6.0] - 2026-08-16

### Changed

- Bound unknown-JWKS-key refreshes with a short negative cache and return safe
  expired/invalid bearer challenge hints without changing the JSON error code.

## [0.5.1] - 2026-08-14

## [0.5.0] - 2026-08-10

## [0.4.0] - 2026-08-10

## [0.3.5] - 2026-08-09

## [0.3.4] - 2026-08-09

## [0.3.3] - 2026-08-09

## [0.3.2] - 2026-08-09

## [0.3.1] - 2026-08-09

## [0.3.0] - 2026-08-09

## [0.2.0] - 2026-08-08

### Added

- Provider-neutral OIDC discovery, cached JWKS verification, internal principals, Axum extraction, and standard authentication/authorization errors.
