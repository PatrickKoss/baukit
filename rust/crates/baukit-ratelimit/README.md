# baukit-ratelimit

`baukit-ratelimit` provides identity-first and client-IP token-bucket limiting
for Axum services. The Redis adapter makes each decision in one atomic Lua
script, so multiple application instances share a consistent bucket. A bounded
in-memory adapter supports tests and Redis-less local development.

Configuration is part of `baukit_config::BaukitConfig` under `rate_limit`.
Environment values therefore follow the normal application-prefixed form, such
as `ORDERS__RATE_LIMIT__REDIS_URL` and
`ORDERS__RATE_LIMIT__IDENTITY__REQUESTS_PER_PERIOD`.

The middleware reads an already verified `baukit_auth::Principal` from request
extensions. Compose authentication outside the rate-limit layer when identity
limiting is enabled. Client IP resolution trusts only the configured number of
rightmost proxy hops and otherwise falls back to Axum `ConnectInfo<SocketAddr>`.

Store failures are fail-open by default and can be configured fail-closed. All
decisions record `http_rate_limit_decisions_total` with only `scope` and
`outcome` labels.

