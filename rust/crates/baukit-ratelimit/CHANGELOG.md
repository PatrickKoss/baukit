# Changelog

All notable changes to `baukit-ratelimit` are documented here.

## [Unreleased]

### Added

- Add `RateLimitOptions::is_enabled` and
  `RedisRateLimitStore::connect_if_enabled`. Fully disabled request limiting no
  longer needs a Redis URL or connection.

### Changed

- Document and test `baukit_auth::establish_principal` as the supported outer
  authentication middleware. Valid identities use separate identity buckets,
  anonymous requests use the IP bucket, and bad credentials consume neither.

### Migration

- Existing `layers` calls remain supported. Applications with custom bearer
  middleware can replace it with `baukit_auth::establish_principal`. Startup
  code can use `connect_if_enabled` instead of checking both scopes itself.

## [0.2.1] - 2026-09-03

### Added

- Add atomic fixed-window amount release for accepted-change accounting in the
  in-memory and Redis stores.

## [0.2.0] - 2026-09-03

### Added

- Add fixed-window amount budgets with in-memory and Redis stores, atomic
  consumption, configurable reset periods, and fail-open or fail-closed error
  handling.

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-ratelimit`.
