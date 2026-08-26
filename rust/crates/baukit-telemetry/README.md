# baukit-telemetry

`baukit-telemetry` installs a service's structured logging, W3C trace propagation, OpenTelemetry
tracer, and Prometheus recorder in one call. `TelemetryBuilder::init` returns a `Telemetry` that
hands out the Prometheus render handle and owns tracer shutdown.

```rust,no_run
use baukit_telemetry::{DeploymentEnvironment, ProcessKind, ServiceIdentity, TelemetryBuilder};

let identity = ServiceIdentity::new(
    "orders",
    ProcessKind::Api,
    env!("CARGO_PKG_VERSION"),
    option_env!("GIT_COMMIT").unwrap_or("unknown"),
    DeploymentEnvironment::Local,
);
let telemetry = TelemetryBuilder::new(identity).init()?;

tracing::info!(message = "service started");
metrics::counter!("domain_operations_total", "operation" => "sync").increment(1);

let exposition = telemetry.prometheus_handle().render();
telemetry.shutdown()?;
# Ok::<(), baukit_telemetry::TelemetryError>(())
```

`tracing`, `metrics`, and `opentelemetry` are re-exported, so products emit ordinary spans and
metrics with no Baukit wrapper in between. There is nothing to learn beyond the upstream crates.

## Initialization is process-global and one-way

A successful `init` cannot be undone. `Telemetry::shutdown` flushes exporters but does not make a
second initialization possible, because the underlying subscriber and recorder registrations are
process-wide.

This has a direct consequence for tests. Every assertion that needs a real recorder, subscriber, or
exporter has to live in a single test in that binary, which initializes telemetry exactly once. Tests
that only need to see log output should install a lightweight subscriber instead, which is what
`baukit_test::init_test_tracing` is for. Discovering this rule from a flaky second test is
unpleasant, so it is stated here rather than only in the rustdoc.

## Label cardinality

Metric label values must be bounded and known at build time. Never label a metric with a path,
identity, token, arbitrary error string, trace ID, request ID, or anything from a provider payload.

Prometheus creates one time series per distinct label combination. A `user_id` label on a metric in a
service with a hundred thousand users is a hundred thousand series, and the failure shows up as an
out-of-memory Prometheus rather than as an obviously bad line of code. Request-scoped values belong on
spans and log lines, where high cardinality is exactly what you want.

## Histogram buckets

The crate owns the recorder, so it owns bucket configuration. `HTTP_DURATION_BUCKETS`,
`DB_POOL_ACQUIRE_DURATION_BUCKETS`, and `WORKER_DURATION_BUCKETS` are exported for the crates that
record those metrics. `baukit-http` and `baukit-ops` record through the `metrics` facade and never
install a recorder of their own, which is what lets one process have one recorder with consistent
buckets.

## Disabling the OpenTelemetry SDK

`OTEL_SDK_DISABLED=true` (case-insensitive) skips the tracer provider, OTLP exporter, span processor,
and exporter background tasks. Any other value, empty included, leaves the SDK on. The programmatic
`TelemetryBuilder::sdk_disabled` wins over the environment variable.

Structured logging and the Prometheus recorder stay active either way. Baukit owns those independently
of the OpenTelemetry SDK, so turning off trace export does not leave a process silent.

## Scope

The crate decides how signals are produced and where they go. It does not decide what a product
measures. Process identity, deployment environment, and log-format vocabulary come from `baukit-core`
and are re-exported here.
