# Changelog

All notable changes to `baukit-ratelimit` are documented here.

## [Unreleased]

### Added

- Initial release: `RateLimitStore` port with Redis (atomic Lua token bucket)
  and in-memory adapters, an Axum layer applying identity-scoped (primary,
  low quota) and client-IP-scoped (safety net, high quota) rate limiting,
  fail-open/fail-closed behavior, `Retry-After`/`RateLimit-*` headers, and the
  `http_rate_limit_decisions_total` metric.
