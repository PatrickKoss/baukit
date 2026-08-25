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

## Redis connection modes

Use a direct URL when the deployment has one Redis replica:

```text
redis://redis.example:6379/
```

Use a Sentinel URL when the deployment has more than one Redis replica. The
comma-separated authorities are Sentinel endpoints and the sole path segment is
the Sentinel master name:

```text
redis+sentinel://sentinel-1:26379,sentinel-2:26379,sentinel-3:26379/mymaster
```

`RedisRateLimitStore::connect` selects the mode from the URL alone. Callers that
already hold the endpoints separately can use
`RedisRateLimitStore::connect_sentinel(["sentinel-1:26379"], "mymaster")`.
Sentinel URLs intentionally do not accept authentication, database paths, query
strings, fragments, or TLS; unsupported forms fail during connection setup
instead of being partially applied.

Sentinel mode resolves and verifies the writable master at startup. If a token
bucket `EVAL` fails, the store asks Sentinel for the master again and retries the
decision once on the newly resolved connection. Requests can still observe a
transient store error while Sentinel is converging; the existing fail-open or
fail-closed middleware policy handles that error unchanged. Clones share the
resolved connection and refresh state.

Store failures are fail-open by default and can be configured fail-closed. All
decisions record `http_rate_limit_decisions_total` with only `scope` and
`outcome` labels.
