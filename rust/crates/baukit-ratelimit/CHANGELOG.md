# Changelog

All notable changes to `baukit-ratelimit` are documented here.

## [Unreleased]

## [0.5.0] - 2026-08-10

### Added

- Redis Sentinel master discovery through `redis+sentinel://` URLs and the
  explicit `RedisRateLimitStore::connect_sentinel` constructor, including a
  single master re-resolution and retry after a failed token-bucket decision.

## [0.4.0] - 2026-08-10

### Added

- Initial release: `RateLimitStore` port with Redis (atomic Lua token bucket)
  and in-memory adapters, an Axum layer applying identity-scoped (primary,
  low quota) and client-IP-scoped (safety net, high quota) rate limiting,
  fail-open/fail-closed behavior, `Retry-After`/`RateLimit-*` headers, and the
  `http_rate_limit_decisions_total` metric.
