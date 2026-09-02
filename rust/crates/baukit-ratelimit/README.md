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

## Fixed-window amount budgets

`FixedWindowAmountBudget` limits units rather than requests. A caller supplies
an opaque subject and the number of units to consume. The budget admits the
whole amount or leaves the counter unchanged. Its decision includes the units
left and the UTC reset instant, which callers can convert to `Retry-After`.

Create `FixedWindowBudgetOptions` with a stable namespace, a limit, a fail
mode, and either `FixedWindow::utc_day()` or an epoch-aligned duration window.
The namespace separates Redis keys and is the only caller-defined metric label.
Do not put subject IDs in it.

`RedisRateLimitStore` checks the limit, increments the counter, and sets its
absolute expiry in one Lua script. `InMemoryRateLimitStore` implements the same
port and accepts the coordinator's clock, which makes boundary tests
deterministic. Store errors return an allowed decision with the full configured
limit in fail-open mode. Fail-closed mode returns a denied decision with zero
remaining units.

Amount-budget decisions record
`fixed_window_amount_budget_decisions_total`. Its labels are `namespace` and
`outcome`. Outcome is one of `allowed`, `denied`, or `error`; subject values are
never metric labels.
