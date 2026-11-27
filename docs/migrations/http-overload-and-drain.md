# Overload shedding, request deadlines and the drain budget

0.32.0 bounds every place an API process could wait without limit: the concurrency queue, the
connection count, outbound calls and the shutdown drain. Most of it works with no code change. The
drain needs a change in every binary, and the `baukit-app` chart needs its Deployments deleted once.

## Queue-timeout shedding

A request that gets no concurrency permit within `HttpOptions::with_queue_timeout` (default 1 s) is
shed with `503 unavailable` and `Retry-After: 1`. Before, it queued until the request timeout. Each shed
request adds one to `http_requests_shed_total{reason="concurrency"}` (`HTTP_REQUESTS_SHED_TOTAL`) and
logs at debug level. Load tests that expected slow `200`s under overload now see `503`s. Clients built
on `@baukit/api-runtime` already retry `503` with `Retry-After` for `GET` and `HEAD`.

## Request deadlines

The lifecycle middleware sets a deadline of start plus `request_timeout` on every request.
`baukit_http::current_deadline()` returns it as a `Deadline` (re-exported from `baukit-resilience`), and
`GuardedClient` caps outbound timeouts with it. To give one route a tighter limit, add the
`route_timeout` layer:

```rust
use baukit_http::route_timeout;

let router = Router::new().route(
    "/v1/search",
    get(search).layer(route_timeout(Duration::from_secs(5))),
);
```

It tightens the deadline too, and on expiry answers the standard `504 request_timeout`. It cannot
extend the global request timeout.

`ApiError::unavailable(retry_after)` is new. It answers `503 unavailable` with `Retry-After` rounded up
to whole seconds, at least 1. Use it when a dependency is down inside a request.

## Connection bounds

Listeners now serve through hyper-util. `ServerOptions` bounds every connection, and the defaults apply
without a code change:

| Setting | Default | Builder |
| --- | --- | --- |
| Header read timeout | 10 s | `with_header_read_timeout` |
| Idle timeout | 60 s | `with_idle_timeout` |
| Open connections | 4096 | `with_max_connections` |

`serve_listener_pair` and `serve_listener_pair_with_shutdown_order` keep their signatures. To change
the bounds, call `serve(listener, router, options, shutdown)` for one listener, or
`serve_listener_pair_with_options` with `ServeOptions { shutdown_order, api, operations }` for the API
and ops pair.

## Drain budget

`shutdown.drain_timeout` defaults to 35 s instead of 30 s. Build the shutdown token from a checked
budget in every binary:

```rust
use baukit_runtime::{DrainBudget, ShutdownToken};

let shutdown = ShutdownToken::with_budget(DrainBudget::new(
    config.shutdown.drain_timeout,
    config.http.request_timeout,
)?);
```

`DrainBudget::new(drain_timeout, request_timeout)` fails with `DrainBudgetError` unless
`request_timeout` plus the 3 s flush reserve (`DEFAULT_FLUSH_RESERVE`) is less than `drain_timeout`.
With the 30 s default request timeout, a drain timeout of 33 s or less now stops the process at startup.
Raise `shutdown.drain_timeout` or lower `http.request_timeout`.

The listener drain, supervised tasks and `run_during_drain` stop when only the reserve is left. Move
the telemetry flush from `run_during_drain` to `run_flush`, the one call that may use the reserve:

```rust
shutdown
    .run_flush(async move { tokio::task::spawn_blocking(move || telemetry.shutdown()).await })
    .await???;
```

A worker passes `WorkerConfig::release_reserve` instead of the request timeout, not its job
timeout: the runner puts attempts that outlive the drain back to pending, so the drain only has
to fit that reserve. `worker_drain_budget` in the generated `backend/crates/<app>-bin/src/lib.rs`
does this.

## The `baukit-app` chart

The chart computes `SHUTDOWN__DRAIN_TIMEOUT` from the grace period:

| Process | Grace default | Drain timeout |
| --- | --- | --- |
| API | `api.terminationGracePeriodSeconds: 45` | grace minus `api.preStopSleepSeconds` (5) minus 5 s, so 35 s |
| Worker | `worker.terminationGracePeriodSeconds: 60` | grace minus 15 s, so 45 s |

Rendering fails when the drain comes out zero or less. A grace period that renders can still fail
`DrainBudget` at startup: with the default request timeout and preStop sleep, the API needs a grace
period of at least 44 s. Delete short `terminationGracePeriodSeconds` overrides from your values.

The API and worker Deployment selectors gain `baukit.dev/workload: <fullname>-<process>`, the label
Flagger uses for canaries. A selector is immutable, so delete both Deployments before the upgrade:

```bash
kubectl -n <namespace> delete deployment <fullname>-api <fullname>-worker
```

Flux or Helm recreates them on the next reconcile. Services, PDBs and topology spread do not select on
the new label.

Other chart defaults change without an action: readiness probes time out after 4 s, the HPA defaults
to `minReplicas: 2`, and the API Ingress retries connection errors twice through a Traefik Middleware
(`ingress.retry`).
