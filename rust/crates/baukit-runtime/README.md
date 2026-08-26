# baukit-runtime

`baukit-runtime` owns what an entire process needs regardless of what it serves: shutdown signaling,
a bounded drain, supervised background tasks, build identity, and binding the public and operations
listeners. Products keep their configuration, routes, and application state.

```rust,no_run
use std::{sync::Arc, time::Duration};

use axum::Router;
use baukit_runtime::{
    ProcessKind, RestartPolicy, ServiceInfo, ShutdownOrder, ShutdownToken, TaskSupervisor,
    build_info, serve_listener_pair_with_shutdown_order,
};
use baukit_telemetry::{DeploymentEnvironment, TelemetryBuilder};
use tokio::net::TcpListener;
# async fn run_outbox_until(_: ShutdownToken) {}
# fn product_routes() -> Router { Router::new() }
# fn operations_routes() -> Router { Router::new() }

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let service = ServiceInfo::new(
        "orders",
        ProcessKind::Api,
        build_info!(),
        DeploymentEnvironment::Local,
    );
    let telemetry =
        Arc::new(TelemetryBuilder::new(service.telemetry_identity().clone()).init()?);

    let shutdown = ShutdownToken::new(Duration::from_secs(30));
    let signal_task = shutdown.spawn_signal_listener();

    let mut workers = TaskSupervisor::new(shutdown.clone());
    let worker_shutdown = shutdown.child_token();
    workers.spawn("outbox", RestartPolicy::Restart { max_restarts: 3 }, move || {
        let worker_shutdown = worker_shutdown.clone();
        async move { run_outbox_until(worker_shutdown).await }
    });

    let api = TcpListener::bind("127.0.0.1:0").await?;
    let ops = TcpListener::bind("127.0.0.1:0").await?;
    let listener_result = serve_listener_pair_with_shutdown_order(
        api,
        Router::new().merge(product_routes()),
        ops,
        Router::new().merge(operations_routes()),
        shutdown.clone(),
        ShutdownOrder::OpsOutlivesApi,
    )
    .await;

    shutdown.trigger();
    let _ = workers.join().await;

    // Telemetry shutdown is synchronous; keep it off the async executor.
    let telemetry_for_flush = Arc::clone(&telemetry);
    shutdown
        .run_during_drain(async move {
            tokio::task::spawn_blocking(move || telemetry_for_flush.shutdown()).await
        })
        .await???;

    signal_task.abort();
    listener_result?;
    Ok(())
}
```

## Draining on a deadline

`ShutdownToken::new` takes the maximum time the process may spend draining, and every piece of
shutdown work shares that one budget. Not a per-task timeout: one deadline for all of it.

An orchestrator sends SIGTERM and then kills the container after its own grace period. A process that
gives each of six shutdown steps thirty seconds can spend three minutes shutting down and get SIGKILLed
partway through, which is exactly the ungraceful exit the sequence was supposed to avoid. Sharing one
deadline means the process finishes what it can and exits on its own terms.

`run_during_drain` runs asynchronous cleanup inside that budget. `on_drain` registers a fast
synchronous hook that fires once on the first `trigger`, before cancellation wakes async work. That
hook is the seam for closing a `baukit_ops::TrafficGate` with no watcher task and no dependency from
runtime onto ops.

## Listener ordering

`ShutdownOrder::Together` stops both listeners at once. `OpsOutlivesApi` keeps operations serving
until the API has drained or the deadline expires.

`OpsOutlivesApi` is the one you usually want. If the ops listener dies first, the orchestrator's
readiness probe starts failing while the API is still finishing in-flight requests, and the events
land in the wrong order: the platform reports the process as unhealthy rather than as shutting down
normally. Keeping ops alive lets the process say "draining" until it is actually done.

Both listeners arrive already bound, so addresses and socket options stay in the composition root. If
either listener fails, shutdown is triggered and the peer is drained.

## Supervised tasks

`TaskSupervisor` ties named background tasks to one shutdown token. `RestartPolicy::FailProcess`
brings the process down as soon as the task stops. `Restart { max_restarts }` restarts up to that many
times, then brings the process down.

There is no infinite-restart option on purpose. A task that keeps dying and keeps restarting is a
service that looks alive to its orchestrator while doing no work, and that is a worse outage than a
crash because nothing escalates. Exhausting the budget triggers shutdown and lets the platform
restart the whole process with the backoff and alerting it already has.

Give each task the `child_token`, not the parent. Triggering a child also triggers the shared process
token, so a task that decides to stop takes the process with it rather than leaving it half-running.

## Build identity

`build_info!` captures version, commit, and Rust version from the crate that invokes it. `GIT_COMMIT`
comes from the build pipeline and falls back to `"unknown"` locally. `ServiceInfo` holds the canonical
`ServiceIdentity` alongside that build metadata, so runtime, logs, metrics, and traces cannot disagree
about which process this is.

## Scope

No product configuration, no telemetry policy, no routes. The crate composes what a process needs and
leaves everything above it alone.
