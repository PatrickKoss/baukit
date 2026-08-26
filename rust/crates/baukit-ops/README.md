# baukit-ops

`baukit-ops` builds the private operations router: liveness, readiness, Prometheus exposition, and
build information. Bind it to a separate listener from your public API.

```rust,no_run
use std::time::Duration;

use baukit_ops::{OpsRouter, ReadinessError, ReadinessRegistry, TrafficGate};
use baukit_telemetry::{DeploymentEnvironment, ProcessKind, ServiceIdentity, TelemetryBuilder};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let identity = ServiceIdentity::new(
    "orders",
    ProcessKind::Api,
    env!("CARGO_PKG_VERSION"),
    option_env!("GIT_COMMIT").unwrap_or("unknown"),
    DeploymentEnvironment::Local,
);
let telemetry = TelemetryBuilder::new(identity.clone()).init()?;

let readiness = ReadinessRegistry::new();
readiness.register_fn("database", Duration::from_secs(2), || async {
    // Ping the database. The error text is returned by /readyz, so keep it public-safe.
    Ok::<_, ReadinessError>(())
})?;
readiness.register_diagnostic_fn_default("upstream_latency", || async {
    Ok::<_, ReadinessError>(())
})?;

let traffic = TrafficGate::new();
let app = OpsRouter::new(identity, telemetry.prometheus_handle().clone())
    .with_readiness(readiness)
    .with_traffic_gate(traffic.clone())
    .into_router();
# let _ = app;
# Ok(())
# }
```

## This router has no authentication

There is no auth and no CORS policy on any of these endpoints, and adding them is not planned.
`/metrics` exposes your entire metric surface and `/readyz` names your dependencies and why they are
failing. Bind the router to a private listener that ingress cannot reach. Mounting it on a public
router is the mistake this crate can neither detect nor prevent.

Readiness error text lands in an HTTP response. `ReadinessError` strips control characters,
normalizes whitespace, and bounds length, but it cannot know which of your strings are secret. Build
messages from static text, never from a provider payload or a raw driver error.

## Endpoints

- `GET /healthz`: process liveness. Answers "is this process running", nothing more.
- `GET /readyz`: every registered check, run concurrently, plus the manual `TrafficGate`.
- `GET /metrics`: renders the `PrometheusHandle` that `baukit-telemetry` created.
- `GET /buildinfo`: service name, version, commit, Rust version.

## Gating versus diagnostics

A gating check that fails makes the process unready and takes it out of the load balancer. A
diagnostic check is reported in the response body and never does that.

The distinction exists because not every dependency should be able to stop your service. A database
you cannot query means you cannot serve requests, so it gates. A slow non-essential upstream is worth
seeing on the endpoint, but making it gate hands that upstream the power to take your whole fleet out
of rotation. Register it with `register_diagnostic_fn_default` and read it on the endpoint instead.

`TrafficGate` is the manual override, for draining a process ahead of a deploy without killing it.

Checks run concurrently and each carries its own timeout, so `/readyz` costs about as long as its
slowest check rather than the sum. `DEFAULT_READINESS_TIMEOUT` is two seconds.

## PostgreSQL pool instrumentation

With the `sqlx-postgres` feature, use `baukit_ops::acquire` in place of `PgPool::acquire` and
`baukit_ops::begin` in place of `PgPool::begin`, then pass `&mut *connection` to queries you would
have run against `&PgPool`. Those record pool wait duration and acquisition timeouts.

Queries handed a bare `&PgPool` bypass acquisition metrics. SQLx exposes no hook that can observe an
implicit executor acquisition, so this is a real limitation rather than an oversight. The periodic
pool gauges from `spawn_pool_metrics_sampler` are unaffected.

## Scope

The crate assembles and serves the endpoints. It does not decide what "ready" means for your product,
own the listener, or install a metrics recorder; `baukit-telemetry` owns the recorder and
`baukit-runtime` binds the listener.
