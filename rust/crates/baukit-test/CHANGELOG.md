# Changelog

All notable changes to `baukit-test` are documented here.

## [Unreleased]

### Added

- Mock OIDC discovery/JWKS server, rotating RS256 token fixture, and authentication-envelope conformance assertions.

### Changed

- JWT fixtures use `ring` directly and no longer select a process-global `jsonwebtoken` crypto provider for consumers.

## [0.1.0] - 2026-08-08

### Added

- Initial private release.
