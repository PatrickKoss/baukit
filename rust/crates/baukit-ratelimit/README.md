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
extensions. Use `baukit_auth::establish_principal` as the outer layer when
identity limiting is enabled:

```rust
use axum::{Router, middleware, routing::get};
use baukit_auth::{AuthState, IdentityVerifier, establish_principal};
use baukit_ratelimit::{InMemoryRateLimitStore, RateLimitOptions, layers};

# fn example(verifier: impl IdentityVerifier + 'static) {
let auth = AuthState::new(verifier);
let app = layers(
    Router::new().route("/", get(|| async { "ok" })),
    InMemoryRateLimitStore::default(),
    RateLimitOptions::default(),
)
.layer(middleware::from_fn_with_state(auth, establish_principal));
# let _ = app;
# }
```

Axum runs the last added layer first. A valid credential therefore selects the
identity bucket. A missing credential uses only the IP bucket. A presented bad
credential returns an authentication response before either bucket is consumed.
Client IP resolution trusts only the configured number of rightmost proxy hops
and otherwise falls back to Axum `ConnectInfo<SocketAddr>`.

`RedisRateLimitStore::connect_if_enabled` returns `None` without parsing the URL
or opening a connection when both scopes are disabled. This lets deployments
turn request limiting off without setting a placeholder Redis URL. An enabled
scope still requires a valid, reachable Redis URL.

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

## Authenticated route groups

`authenticated_route_group` applies another token bucket to selected requests
inside an authenticated router. Construct `AuthenticatedRouteGroupOptions` with
a bounded group name and quota. The options reuse the global key prefix and
fail mode. The application supplies a principal key function and a request
predicate:

```rust
use std::time::Duration;

use axum::{Router, extract::Request, http::Method};
use baukit_auth::Principal;
use baukit_ratelimit::{
    AuthenticatedRouteGroupOptions, InMemoryRateLimitStore, Quota,
    RateLimitOptions, authenticated_route_group,
};

let rate_limit = RateLimitOptions::default();
let writes = AuthenticatedRouteGroupOptions::new(
    "item_writes",
    Quota::new(30, Duration::from_secs(60), 0)?,
    &rate_limit,
)?;
let app: Router = authenticated_route_group(
    Router::new(),
    InMemoryRateLimitStore::default(),
    writes,
    |principal: &Principal| principal.subject().to_owned(),
    |request: &Request| request.method() == Method::POST,
);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Group keys use `group:<name>:` and cannot collide with global `id:` or `ip:`
keys. The group name is the metric `scope`; the extracted subject key is never
a label. Rejections include `Retry-After`, `RateLimit-Limit`,
`RateLimit-Remaining`, and `RateLimit-Reset`. The JSON error details contain the
same whole-second `retry_after` value as the `Retry-After` header.

Place principal-establishing middleware outside the global layer and place the
global layer outside route groups. Axum runs the last added layer first:

```text
establish principal -> global identity/IP limit -> authenticated group -> route
```

`SharedRateLimitStore` wraps any adapter that implements `RateLimitStore` and
`AmountBudgetStore`. Use it when startup selects between the Redis and in-memory
adapters and both request limits and fixed-window amount budgets need the same
selection.

## Migration

Existing `layers` calls keep their behavior. Replace product-owned bearer
middleware with `baukit_auth::establish_principal` and place it outside
`baukit_ratelimit::layers`. Startup code that conditionally connects Redis can
replace its scope checks with `RedisRateLimitStore::connect_if_enabled`. Rate
limit rejections now add numeric `details.retry_after`; their status, code,
message, and headers remain unchanged. Applications can delete response
normalizers that only copied the retry delay into the standard error body.

## Fixed-window amount budgets

`FixedWindowAmountBudget` limits units rather than requests. A caller supplies
an opaque subject and the number of units to consume. The budget admits the
whole amount or leaves the counter unchanged. Call `release` to return units
that the product did not accept after reserving a larger amount. Decisions
include the units left and the UTC reset instant, which callers can convert to
`Retry-After`.

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
`outcome`. Outcome is one of `allowed`, `denied`, `released`, or `error`;
subject values are never metric labels.
