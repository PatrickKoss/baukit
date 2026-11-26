# Resilience contract

**Applies to:** every hop a baukit product makes: browser or app to API, API to provider, worker to peer,
and process to its own dependencies at startup and shutdown.
**Related:** [integration reliability](./integration-reliability.md), [telemetry specification](./telemetry-spec.md),
and the `baukit-resilience` [README](../../rust/crates/baukit-resilience/README.md).

This page states the rules the crates, packages and templates follow when something they depend on is
slow or down. Every number below is a default in code, and each one names the file and constant that
holds it. When the code changes, the code wins and this page is wrong; fix the page.

## The short version

1. A request that cannot succeed right now answers `503` with `Retry-After`. A `500` means a bug.
2. Whoever receives a `Retry-After` waits at least that long before calling the same origin again.
3. A call path has exactly one retry layer. Everything below it fails fast and reports why.
4. Every retry is bounded and uses full jitter.
5. Outbound calls go through `baukit_egress::GuardedClient`, which holds a circuit breaker per
   `(scheme, host, port)` and labels metrics by a static destination name.
6. Read-mostly data (signing keys, entitlements, hub tokens) is served stale for a bounded window when
   its source fails.
7. A process waits for its dependencies at startup inside a budget, and drains inside a budget at
   shutdown. Neither budget can be larger than the Kubernetes window around it.

## Who returns 503

| Condition | Answer | Where |
|---|---|---|
| No concurrency permit within the queue timeout | `503`, `Retry-After: 1`, `http_requests_shed_total{reason="concurrency"}` | `baukit-http/src/shedding.rs` `SHED_RETRY_AFTER`; queue timeout `baukit-http/src/options.rs` `DEFAULT_QUEUE_TIMEOUT` (1 s); permits `DEFAULT_CONCURRENCY_LIMIT` (1024) |
| Rate-limit store down, fail mode `closed` | `503`, `Retry-After: 1` | `baukit-ratelimit/src/axum_layer.rs` and `src/browser_session.rs` `STORE_UNAVAILABLE_RETRY_AFTER` |
| Identity verification unavailable (JWKS gone past the stale window, issuer down) | `503`, `Retry-After: 5` | `baukit-auth/src/axum_integration.rs` `UNAVAILABLE_RETRY_AFTER_SECONDS` |
| MCP token introspection unavailable or its circuit open | `503`, `Retry-After: 5` | `baukit-mcp/src/security.rs` `IDENTITY_UNAVAILABLE_RETRY_AFTER_SECONDS` |
| Browser-session IdP unavailable or its circuit open | `503 idp_unavailable`, `Retry-After` from the open circuit or full bulkhead, else 5; session kept | `baukit-auth/src/browser_session/router.rs` `IDP_UNAVAILABLE_RETRY_AFTER` |
| Entitlements past `grace_until` while the hub fails | `503`, `Retry-After` from the open breaker or the remaining cooldown (10 s) | `baukit-suite/src/platform/cache.rs` `ENTITLEMENT_FAILURE_COOLDOWN` |
| Plan entitlements or credits unavailable | `503`, `Retry-After: 5` | `baukit-suite/src/platform/plan.rs` `PLAN_RETRY_AFTER_SECONDS` |
| Suite service unavailable | `503`, `Retry-After: 300` | `baukit-suite/src/domain/mod.rs` `SUITE_UNAVAILABLE_RETRY_SECONDS` |
| Failed-job requeue store error on the ops port | `503`, `Retry-After: 5` | `baukit-ops/src/jobs_admin.rs` `STORE_RETRY_AFTER` |
| Generated API: database or pool cannot serve | `503`, `Retry-After: 5` | `templates/backend/backend/crates/__app__-api/src/lib.rs` `REPOSITORY_RETRY_AFTER` |

Build every other transient outage answer with `ApiError::unavailable(retry_after)`
(`baukit-http/src/error.rs`). It rounds the delay up to whole seconds, with a floor of 1 second.

A request that runs out of time answers `504 request_timeout` without `Retry-After`. The global limit is
`baukit-http/src/options.rs` `DEFAULT_REQUEST_TIMEOUT` (30 s), and `baukit_http::route_timeout` can
only tighten it for one route. A real rate limit still answers `429` with the limiter's `Retry-After`.

## Who honours Retry-After

| Caller | What it does | Bound |
|---|---|---|
| `GuardedClient` (Rust, outbound) | Classifies `429` and `5xx` with `Retry-After` as `RetryClass::RetryAfter`. The breaker then holds the circuit open for exactly that delay, without counting a failure (D4). | Clamped to `baukit-egress/src/options.rs` `DEFAULT_MAX_RETRY_AFTER` (300 s) |
| `WorkerRunner` (Rust, jobs) | `JobError::retryable_after` sets `run_after` to the hint. An open circuit becomes `JobError::deferred` through `egress_job_error`. | `max_attempts`, and `WorkerConfig::max_deferrals` (100) for deferrals |
| `@baukit/api-runtime` | Retries a `GET` or `HEAD` answered `429` or `503` with `Retry-After` after the delay, and blocks the origin in the cooldown until then. | `api-runtime/src/retry.ts` `DEFAULT_MAX_RETRY_AFTER_MS` (30 s) for a retry; `api-runtime/src/cooldown.ts` `DEFAULT_MAX_COOLDOWN_MS` (300 s) for the cooldown |
| `@baukit/sync-client` | The default retry policy waits until `SyncRateLimitError.retryAt`. | `sync-client/src/scheduler-policy.ts` `DEFAULT_SYNC_MAX_RETRIES` (5) |
| Generated web and mobile apps | `ApiUnavailableError.retryAt` drives the "unavailable, retrying at HH:MM" banner. | `templates/web/web/src/api-availability.ts` |

## One retry layer per call path

Retries stack. Two layers of three retries are nine calls into a service that is already down. Each
call path picks one layer that retries, and every layer below it fails fast with a classified error.

| Call path | The retry layer | Layers that do not retry |
|---|---|---|
| Browser to API | `@baukit/api-runtime` (`GET`/`HEAD` by default) | TanStack Query in the web template (`retry: false`, no refetch on window focus, `templates/web/web/src/query-client.ts` `createAppQueryClient`); Traefik retries only when it cannot reach a pod |
| Mobile token renewal | `scheduleTokenRenewal` in `@baukit/auth-native` | the refresh request itself |
| Offline sync | the `@baukit/sync-client` scheduler | the product's transport |
| API to provider inside a request | none: the request fails with `503` and the client retries | `GuardedClient` never retries |
| Job to provider | `WorkerRunner` attempts and deferrals | `GuardedClient`, the job handler |
| Process to dependency at startup | `baukit_resilience::startup::retry_within` | the connect call |

`GuardedClient` has no retry loop on purpose. It returns `EgressError` with a `retry_class()`, and the
caller decides. Inside an inbound request, that decision is almost always to return `503`.

The Traefik `retry` middleware on the API route (2 attempts, 100 ms initial interval, in the
`baukit-app` chart values `ingress.retry`) only fires when no pod accepted the connection, so it does
not add a layer on top of the app. There is no edge circuit breaker (D8).

## Default policies

### Backoff

All backoff in Rust and TypeScript is full jitter: `floor(min(max, initial * 2^n) * random)`. Both sides
replay `fixtures/resilience/backoff-v1.json`.

| Where | Initial | Max | Retries | Constants |
|---|---|---|---|---|
| Startup connects | 250 ms | 5 s | until the startup budget | `baukit-resilience/src/startup.rs` `RETRY_INITIAL`, `RETRY_MAX` |
| Job retries | 1 s | 300 s | `max_attempts` of the job | `baukit-jobs/src/runner.rs` `WorkerConfig::default` `retry_initial`, `retry_max` |
| Job store errors in the runner loop | `poll_interval` (250 ms) | 30 s | until shutdown | `baukit-jobs/src/runner/backoff.rs` `STORE_RETRY_CAP` |
| Redis rate-limit reconnect after a degraded start | 500 ms | 30 s | until connected | `baukit-ratelimit/src/redis_store/startup.rs` `RECONNECT_INITIAL`, `RECONNECT_MAX` |
| `@baukit/api-runtime` | 100 ms | 2 s | 2 | `api-runtime/src/retry.ts` `DEFAULT_BASE_DELAY_MS`, `DEFAULT_MAX_DELAY_MS`, `DEFAULT_MAX_RETRIES` |
| `@baukit/sync-client` | 1 s | 60 s | 5 | `sync-client/src/scheduler-policy.ts` `DEFAULT_BASE_DELAY_MS`, `DEFAULT_MAX_DELAY_MS`, `DEFAULT_SYNC_MAX_RETRIES` |
| `scheduleTokenRenewal` | 1 s | 60 s | until a terminal refresh error | `auth-native/src/token-renewal.ts` `DEFAULT_BASE_DELAY_MS`, `DEFAULT_MAX_DELAY_MS` |

The sync scheduler also spreads recovery. Online and foreground triggers wait a random 0 to 5 s
(`RECOVERY_JITTER_MAX_MS`), and interval ticks land within 10% of the interval
(`INTERVAL_JITTER_RATIO`), so clients that lost the backend together do not return together.

### Circuit breaker (Rust)

`baukit-resilience/src/breaker.rs` `BreakerPolicy::default`, applied by `GuardedClient` unless the
options call `without_circuit_breaker()`:

| Field | Default | Meaning |
|---|---|---|
| `window` | 20 | recent calls the failure rate covers |
| `min_calls` | 10 | calls needed before the rate can open the breaker |
| `failure_rate_percent` | 50 | failure rate that opens it |
| `consecutive_timeouts` | 5 | timeouts in a row that open it regardless of the window |
| `open_base` | 30 s | first open time, doubled on each re-open |
| `open_max` | 300 s | cap for any open time, `Retry-After` holds included |
| `jitter_percent` | 20 | up to this share is taken off each open time |
| `probe_timeout` | 30 s | a half-open probe silent for this long is abandoned |

Half-open admits one probe. A success closes the breaker and a failure re-opens it. Every outcome is
tied to the permit it was admitted with, so a late answer from before the circuit opened cannot change
its state.

What counts (D4, `baukit-egress/src/breaker.rs` `status_outcome` and `error_outcome`): transport and DNS
failures and `5xx` without `Retry-After` are failures, timeouts are timeouts, and `429` or `5xx` with
`Retry-After` hold the circuit for that delay. `4xx`, revoked credentials and refused destinations never
count.

A timeout only counts when the caller's deadline left the destination a fair chance: at least half the
client's request timeout (`FAIR_BUDGET_DIVISOR`). A caller with less than 50 ms left fails at once with
`EgressError::Timeout` and sends nothing (`baukit-egress/src/client.rs` `MIN_DEADLINE_REMAINING`).

Each `GuardedClient` also caps requests in flight per `(scheme, host, port)` at 32
(`baukit-egress/src/options.rs` `DEFAULT_MAX_IN_FLIGHT`). A full bulkhead returns
`EgressError::Saturated`, whose retry class asks for 1 s (`baukit-egress/src/breaker.rs`
`SATURATED_RETRY_AFTER`). Breaker and bulkhead state is per process and never shared through Redis (D6).

### Outbound timeouts

`baukit-egress/src/options.rs`: `DEFAULT_RESOLVE_TIMEOUT` 3 s, `DEFAULT_CONNECT_TIMEOUT` 5 s,
`DEFAULT_REQUEST_TIMEOUT` 10 s. Inside an inbound request, the request deadline set by `baukit-http`
caps the request timeout, so an API call with 2 s left never waits 10 s for a provider. Clients with
their own limits: `baukit-erasure/src/egress.rs` `CONNECT_TIMEOUT` 2 s and `REQUEST_TIMEOUT` 5 s for
Keycloak admin, `baukit-mcp/src/introspection.rs` `DEFAULT_TIMEOUT` 2 s, `baukit-push/src/config.rs`
`DEFAULT_REQUEST_TIMEOUT` 8 s, `baukit-auth/src/browser_session/config.rs` `DEFAULT_REQUEST_TIMEOUT`
10 s.

### Browser cooldown (TypeScript)

`@baukit/api-runtime` keeps a cooldown per origin instead of a full breaker (D9). Constants in
`api-runtime/src/cooldown.ts`:

- A `429` or `503` with `Retry-After` blocks the origin for that delay.
- `DEFAULT_NETWORK_FAILURE_THRESHOLD` (5) network failures or timeouts in a row block it for
  `DEFAULT_NETWORK_FAILURE_COOLDOWN_MS` (30 s) minus up to `DEFAULT_JITTER_PERCENT` (20%).
- Each further run of failures doubles the block, up to `DEFAULT_MAX_COOLDOWN_MS` (300 s).
- Any HTTP response resets the count.

While an origin is blocked, calls reject with `ApiUnavailableError` and its `retryAt` without sending a
request. The cooldown lives in one runtime instance, so an app shares one runtime for all API calls.

Each attempt has a timeout of `DEFAULT_REQUEST_TIMEOUT_MS` (30 s) in `api-runtime/src/fetch.ts`. Other
client timeouts: the browser-session refresh POST aborts after `REFRESH_TIMEOUT_MS` (10 s,
`auth-web/src/browser-session-client.ts`) so a hanging backend cannot hold the cross-tab lock, and
`pwa-web` `networkFirst` serves the cached copy after `DEFAULT_NETWORK_TIMEOUT_MS` (4 s,
`pwa-web/src/fetch-handler.ts`).

## Destinations

Every `GuardedClient` takes a destination name as its first argument. The name is the `destination`
label on `egress_requests_total`, `egress_request_duration_seconds`, `egress_circuit_state` and
`egress_circuit_transitions_total`. The breaker key is still the `(scheme, host, port)`, so one dead peer
behind a shared client does not block the others.

| Name | Client | Where it is built |
|---|---|---|
| `oidc` | OIDC discovery and JWKS | `baukit-auth/src/oidc_http.rs` |
| `idp` | browser-session discovery and token endpoint | `baukit-auth/src/browser_session/idp.rs` |
| `hub` | suite hub API, tokens and ticket keys | `baukit-suite/src/platform/hub.rs`, `templates/platform/backend/crates/__app__-api/src/platform.rs` |
| `suite-peer` | suite deliveries to peer apps | `baukit-suite/src/adapters/peer.rs` |
| `expo` | Expo push send and receipts | `baukit-push/src/expo.rs` |
| `keycloak-admin` | identity deletion | `baukit-erasure/src/egress.rs` |
| `mcp-introspection` | MCP token introspection | `baukit-mcp/src/introspection.rs` |

### Naming a new destination

Build one client per provider and name it in code:

```rust
let payments = GuardedClient::new("payments", EgressOptions::default())?;
```

The name is short, lowercase, stable and static. Never put a host, tenant, user or URL in it, because
each name becomes a metric label value. Two clients for the same provider share the name. The
`templates/backend/docs/outbound-providers.md` page in every generated product repeats this.

### Mapping an open circuit in a job handler

An open circuit or a full bulkhead means the provider is down or busy, not that the job is wrong. Turn
on the `egress` feature of `baukit-jobs` and map the error with `egress_job_error`
(`baukit-jobs/src/egress.rs`):

```rust
use baukit_jobs::egress_job_error;

let response = payments
    .execute(request)
    .await
    .map_err(|error| egress_job_error(&error))?;
```

| `EgressError` | `JobError` |
|---|---|
| `CircuitOpen { retry_after }` | `deferred` for `retry_after` |
| `Saturated` | `deferred` for 1 s (`SATURATED_DEFERRAL`) |
| not retryable (blocked address, `4xx`) | `permanent` |
| retryable with a `Retry-After` | `retryable_after` that delay |
| other retryable | `retryable` with job backoff |

A deferral puts the job back to `pending` without consuming an attempt. Past `WorkerConfig::max_deferrals`
(100) a deferral consumes an attempt like `retryable_after`, so a destination that never comes back
still ends in `failed` (D7). Deferrals are counted in `worker_job_deferrals_total{job_kind}`.

## Stale data

| Data | Fresh | Served stale | Failure cooldown | Constants |
|---|---|---|---|---|
| OIDC JWKS | 300 s | 1 h for known key ids | 10 s; an unknown key id forces at most one refresh per cooldown | `baukit-auth/src/config.rs` `DEFAULT_JWKS_FRESH_FOR`, `DEFAULT_JWKS_STALE_FOR`; `baukit-auth/src/verifier/keys.rs` `FAILURE_COOLDOWN` |
| Suite ticket keys | 300 s | 1 h | 5 s | `baukit-suite/src/platform/tickets.rs` `JWKS_TTL`, `JWKS_STALE_FOR`, `REFRESH_COOLDOWN` |
| Hub tokens | until 30 s before expiry | no | 5 s | `baukit-suite/src/platform/hub.rs` `TOKEN_REFRESH_MARGIN`, `TOKEN_FAILURE_COOLDOWN` |
| Entitlements | until `valid_until` | until `grace_until` from the snapshot, marked `stale: true` | 10 s | `baukit-suite/src/platform/cache.rs` `ENTITLEMENT_FAILURE_COOLDOWN` |

A stale key set never verifies a key id it does not contain. One background refresh runs at a time, and
no request waits on it. `auth_key_cache_requests_total{source,result}` shows `fresh`, `stale`, `fetched`
and `failed` lookups for `oidc` and `ticket`.

## Startup

A process waits for its dependencies with backoff instead of exiting on the first failed connect.

- The budget is `startup.budget` in config, default 60 s (`baukit-config/src/startup.rs`
  `DEFAULT_STARTUP_BUDGET`). `OidcConfig` and `BrowserSessionConfig` have the same 60 s default
  (`baukit-auth/src/config.rs` `DEFAULT_STARTUP_BUDGET`).
- PostgreSQL: `baukit_ops::connect_with_backoff`, and `connect_for_migrations` for migration binaries.
  Wrong credentials, a missing database, TLS and URL errors fail at once.
- Redis rate-limit store: `RedisRateLimitStore::connect_if_enabled` returns a degraded store after the
  budget instead of failing. It reconnects in the background, and until then the rate-limit fail mode
  decides every request.
- OIDC: discovery retries inside the budget. `OidcConfig::with_jwks_uri` skips discovery, so the API
  starts without the issuer.
- The chart's startup probe allows 150 s (`periodSeconds` 5 times `failureThreshold` 30 in
  `deploy/chart/baukit-app/values.yaml`), so a 60 s budget ends before Kubernetes restarts the pod.

Once running, a process does not exit on a dependency outage. It reports not ready and answers `503`.
The readiness check times out after 2 s (`baukit-ops/src/lib.rs` `DEFAULT_READINESS_TIMEOUT`), below the
4 s probe `timeoutSeconds`, so a slow database reads as not ready rather than a probe timeout. The
worker's readiness reports `WorkerRunner::store_health()`.

## Drain

API, in the order of a rollout:

1. The preStop hook sleeps 5 s while endpoints drop (`api.preStopSleepSeconds`).
2. The drain timeout is the grace period (45 s) minus the preStop sleep minus 5 s of slack, computed by
   `baukit-app.apiDrainSeconds` in `deploy/chart/baukit-app/templates/_helpers.tpl`. The chart refuses
   to render when that is zero or less. The default leaves 35 s, which matches
   `baukit-config/src/lib.rs` `DEFAULT_DRAIN_TIMEOUT`.
3. `DrainBudget::new(drain_timeout, request_timeout)` refuses a drain timeout that cannot fit one full
   request (30 s) plus the flush reserve of 3 s (`baukit-runtime/src/budget.rs` `DEFAULT_FLUSH_RESERVE`).
   The process fails at startup instead of losing its last spans to SIGKILL.
4. The listener drain and supervised tasks stop when only the reserve is left. `run_flush` alone may use
   the reserve.
5. `ShutdownOrder::OpsOutlivesApi` keeps the ops listener up until the API has drained, so `/readyz`
   reports "service is draining" instead of refusing connections.

Connection limits (`baukit-runtime/src/server.rs`): `DEFAULT_HEADER_READ_TIMEOUT` 10 s,
`DEFAULT_IDLE_TIMEOUT` 60 s, `DEFAULT_MAX_CONNECTIONS` 4096. Traefik closes idle upstream connections
after 5 s, below the 10 s header timeout, so it never reuses a connection the API already closed.

Worker: the drain timeout is the grace period (60 s) minus a 15 s reserve (`baukit-app.workerDrainSeconds`).
The runner stops claiming, cancels attempts, waits until `WorkerConfig::release_reserve` (5 s) before the
deadline, then releases unfinished attempts back to `pending` without consuming an attempt. A lease
lasts 60 s (`lease_duration`) and running attempts renew it every third of that, so a killed worker
strands its jobs for at most one lease.

PostgreSQL session timeouts keep a dropped request from holding a connection
(`baukit-config/src/database.rs`): `DEFAULT_STATEMENT_TIMEOUT` 29 s, below the request timeout, which
config validation enforces; `DEFAULT_LOCK_TIMEOUT` 5 s; `DEFAULT_IDLE_IN_TRANSACTION_SESSION_TIMEOUT`
60 s; pool `DEFAULT_ACQUIRE_TIMEOUT` 5 s.

## Fail open or fail closed

| Dependency down | Behaviour | Why |
|---|---|---|
| Rate-limit store, `rate_limit.fail_mode = "open"` (the default in `baukit-config/src/lib.rs`) | requests pass unlimited | a limiter outage should not take the API down |
| Rate-limit store, `fail_mode = "closed"` | `503`, `Retry-After: 1` | for routes where an unlimited caller is worse than an outage |
| Identity provider, inside the stale window | known keys keep verifying | principle 4 |
| Identity provider, past the stale window or unknown key id | closed: `503` or `401` | a token is never accepted without a key that verifies it |
| Hub, inside the entitlement grace window | the last snapshot is served, `stale: true` | the hub sets `grace_until` for this |
| Hub, past `grace_until` | closed: `503` | entitlements gate paid features |
| Provider circuit open | closed for that destination: `503` in a request, a deferral in a job | the provider said it is down |
| Database | closed: `503` from the repository, readiness off | there is nothing to serve |

## Changing a default

Change the constant, then this page in the same commit. A new outbound client needs a destination name
from the rules above. A new `503` needs `ApiError::unavailable` with a delay that says when to come
back. A new retry loop needs a reason why the layer above cannot do the retrying.
