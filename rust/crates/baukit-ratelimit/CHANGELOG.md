# Changelog

All notable changes to `baukit-ratelimit` are documented here.

## [Unreleased]

- Fixed: an outer rate-limit layer no longer overwrites the `RateLimit-*` headers an inner layer already set, so nested identity and IP layers report the identity quota.

## [0.31.0] - 2026-11-26

- Breaking: `trusted_proxy_hops` defaults to 0, only the trusted `X-Forwarded-For` entry is parsed, IPv6 keys bucket per /64 (`ipv6_prefix_len`), the IP scope returns 500 without `ConnectInfo`, `AmountBudget::release` takes the consume's `BudgetWindow` (whose `index()` is `i64`), `FixedWindowError::ZeroDuration` becomes `TooShort` plus a new `TooLong`, and the Redis token bucket uses new hash fields. See `docs/migrations/rate-limit-client-ip-and-buckets.md`.

## [0.30.6] - 2026-11-26

## [0.30.5] - 2026-11-26

## [0.30.4] - 2026-11-25

## [0.30.3] - 2026-11-24

## [0.30.2] - 2026-11-24

## [0.30.1] - 2026-11-24

## [0.30.0] - 2026-11-23

## [0.29.0] - 2026-11-23

## [0.28.8] - 2026-11-22

## [0.28.7] - 2026-11-22

## [0.28.6] - 2026-11-22

- `RedisRateLimitStore` can prepend a key namespace to every key it reads or writes, in `check_and_consume`, `check_and_consume_amount` and `release_amount`. `connect_if_enabled` takes it from the new `rate_limit.redis_key_namespace` (empty by default, so existing keys stay where they are); `with_key_namespace` sets it on a store built by hand. This also covers keys that do not use `rate_limit.key_prefix`, such as `amount-budget:`, `suite:` and `mcp:`. Products that share one Redis set a distinct namespace each and give each a Redis ACL user restricted to `~<namespace>*`; the README lists the commands that user needs.
- `redis+sentinel://` URLs accept `user:password@` or `:password@` before the Sentinel hosts. The credentials authenticate to the discovered data nodes; Sentinel nodes are still contacted without credentials. Before, any `@` in a Sentinel URL was rejected.

## [0.28.5] - 2026-11-22

## [0.28.4] - 2026-11-19

## [0.28.3] - 2026-11-19

## [0.28.2] - 2026-11-18

## [0.28.1] - 2026-11-18

## [0.28.0] - 2026-11-17

## [0.27.4] - 2026-11-17

## [0.27.3] - 2026-11-17

## [0.27.2] - 2026-11-17

## [0.27.1] - 2026-11-17

## [0.27.0] - 2026-11-17

## [0.26.7] - 2026-11-17

## [0.26.6] - 2026-11-17

## [0.26.5] - 2026-11-17

## [0.26.4] - 2026-11-16

## [0.26.3] - 2026-11-16

## [0.26.2] - 2026-11-16

## [0.26.1] - 2026-11-16

## [0.26.0] - 2026-11-16

## [0.25.2] - 2026-11-13

## [0.25.1] - 2026-11-13

## [0.25.0] - 2026-11-13

## [0.24.0] - 2026-11-12

## [0.23.0] - 2026-11-12

## [0.22.0] - 2026-11-11

## [0.21.0] - 2026-11-10

## [0.20.0] - 2026-11-10

## [0.19.2] - 2026-11-08

## [0.19.1] - 2026-11-08

## [0.19.0] - 2026-11-07

- New `browser-session` feature: `StoreBrowserSessionRateLimiter` implements baukit-auth's `BrowserSessionRateLimiter` over any `RateLimitStore`, keyed per client IP (honouring `trusted_proxy_hops`) and per session id, following `fail_mode`.
- Strict workspace clippy lint set (QS W1-A).

## [0.18.3] - 2026-11-07

## [0.18.2] - 2026-11-06

## [0.18.1] - 2026-11-05

## [0.18.0] - 2026-11-05

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

## [0.15.0] - 2026-11-02

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

## [0.13.0] - 2026-10-31

## [0.12.2] - 2026-10-31

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

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
