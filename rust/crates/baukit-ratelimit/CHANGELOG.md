# Changelog

All notable changes to `baukit-ratelimit` are documented here.

## [Unreleased]

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

## [0.9.0] - 2026-10-07

- Expose AuthenticatedRouteGroupOptions::key so REST and external callers can share a quota without duplicating key formatting.

- Trigger Sentinel promotion explicitly in the recovery test and keep its existing deadline under CPU load.

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

## [0.6.0] - 2026-10-02

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Changed

- Break: the rejection detail `retry_after` is now `retryAfter`. The `Retry-After` header is
  unchanged.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

### Added

- Add validated authenticated route-group limits with caller-supplied subject
  keys and request predicates. Group counters have their own namespace and
  rejections return all rate-limit headers.
- Add `SharedRateLimitStore` so one runtime-selected adapter can back request
  limits and fixed-window amount budgets without an application delegation
  enum.
- Add `RateLimitOptions::is_enabled` and
  `RedisRateLimitStore::connect_if_enabled`. Fully disabled request limiting no
  longer needs a Redis URL or connection.

### Changed

- Add numeric `details.retry_after` to standard rate-limit rejection bodies.
  The value matches the `Retry-After` header.
- Document and test `baukit_auth::establish_principal` as the supported outer
  authentication middleware. Valid identities use separate identity buckets,
  anonymous requests use the IP bucket, and bad credentials consume neither.

### Migration

- Existing `layers` calls and error codes remain supported. Clients may read
  `error.details.retry_after` instead of parsing text. Applications with route
  wrappers can replace their key construction, store error mapping, response
  headers, and retry detail normalization with `authenticated_route_group`.
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
