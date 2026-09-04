# Evidence for principal-establishing middleware

## Source product files

- `leitbild/backend/crates/leitbild-api/src/lib.rs`: `rate_limited_router` and
  `cache_presented_principal`.
- `leitbild/backend/crates/leitbild-bin/src/lib.rs`: `connect_rate_limiter`.
- `eigenruhe/backend/crates/eigenruhe-api/src/lib.rs`: the `auth::authenticate`
  middleware and split identity and IP layer composition.
- `eigenruhe/backend/crates/eigenruhe-bin/src/bin/api.rs`: conditional Redis
  store selection in `api_rate_limit_store`.

## Observed repeated glue

Both products call the `Principal` extractor in outer custom middleware so the
inner rate limiter can read request extensions. Without that step, the route
extractor verifies too late and authenticated requests reach the limiter without
an identity. Leitbild also checks enabled scopes before opening Redis. Eigenruhe
keeps its own store-selection function.

## Baukit owner

`baukit-auth` owns bearer verification and principal caching.
`baukit-ratelimit` owns bucket selection and conditional Redis connection.

## Public types and errors

- `baukit_auth::establish_principal` uses `AuthState`, `Principal`, and the
  existing `AuthRejection` envelope.
- `RateLimitOptions::is_enabled` reports whether a request store is needed.
- `RedisRateLimitStore::connect_if_enabled` returns
  `Result<Option<RedisRateLimitStore>, RateLimitStoreError>`.

## Product-owned inputs

Products still choose the `IdentityVerifier`, issuers, token stores, quotas,
fail mode, trusted proxy count, Redis URL, and which routes allow anonymous
traffic.

## Required cases

- Concurrency: one verified principal is stored per request and reused by route
  extraction. Store implementations retain their atomic decision contract.
- Failure: invalid and expired credentials stop before rate limiting. Store
  errors retain fail-open and fail-closed behavior.
- Privacy: raw credentials are not stored in extensions, logs, metrics, or rate
  limit keys.
- Cleanup: request extensions end with the request. Disabled scopes open no
  Redis connection.

## Supported runtimes

Axum services on Baukit's Rust MSRV, with in-memory or Redis rate-limit stores.

## Product adoption change

Leitbild can delete `cache_presented_principal` and use the Baukit middleware in
`rate_limited_router`. Eigenruhe can delete `auth::authenticate` and replace its
split authentication composition with the supported outer middleware. Both can
replace local Redis bootstrap checks with `connect_if_enabled` after a released
Baukit version is adopted.
