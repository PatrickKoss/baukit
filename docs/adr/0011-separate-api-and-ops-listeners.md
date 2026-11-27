# ADR 0011: API and operations endpoints use separate listeners, with a traffic gate for drain

## Status
Accepted, 2026-11-27.

## Context

Health checks and metrics must not be reachable from the internet. Metrics reveal internals, and a
public readiness endpoint lets anyone probe deploy state. Sharing one port also puts probes behind
the API's rate limiter and auth.

On shutdown, the load balancer must stop sending traffic before the API stops accepting it.
Otherwise in-flight requests fail on every deploy.

## Decision

- `OpsRouter` in `baukit-ops` serves `/healthz`, `/readyz`, `/metrics` and `/buildinfo` on its own
  listener. `OpsConfig` defaults it to `0.0.0.0:9090`. The API listener defaults to port 8080.
- `TrafficGate` starts open. `stop_accepting` makes `/readyz` fail, so the platform removes the pod
  before the drain starts.
- `serve_listener_pair_with_shutdown_order` in `baukit-runtime` serves both listeners.
  `ShutdownOrder::OpsOutlivesApi` keeps ops serving until the API has drained or the
  drain deadline expires. `on_drain` registers hooks that run during the drain. `ShutdownOrder::Together` stops both at once.
- The chart `deploy/chart/baukit-app` exposes ops as a `ClusterIP` service on 9090, and its network
  policy lets only Prometheus reach it.

## Enforced by

- `rust/crates/baukit-ops/src/tests.rs`: `traffic_gate_flip_fails_readiness_before_shutdown`.
- The `security` rule of `baukit check`: `ops_service_must_be_cluster_ip` fails on any other
  service type.
- Nothing rejects an ops port equal to the API port. A config validation in `baukit-config` would.

## Consequences

Products get probes and metrics with no exposure and a drain that does not drop requests. They run
two ports and must keep the ops port out of any ingress.

## History

- 2026-08-25, commit `af6e2b4f` (baukit 0.1.0): separate listeners and `TrafficGate`.
- 2026-11-25, commit `a8dad46e`: `baukit check` started rejecting a non-ClusterIP ops service.
