# ADR 0029: Every baukit backend emits the same resource attributes and fixed metric names, and shipped dashboards may query only those names

## Status

Accepted, 2026-11-27.

## Context

Products built before baukit each named their HTTP metrics differently: plural
and singular duration names, status classes against raw codes, raw paths
against route templates, `service_name` against `service` in Loki. A shared
dashboard or alert then works for one product and shows nothing for the next.

Baukit ships dashboards, alerts and recording rules under
`deploy/observability/` and `deploy/observability-platform/`. Those only work
if every backend emits the names they query, and if a dashboard edit cannot
reference a name no backend emits.

## Decision

The [telemetry specification](../platform/telemetry-spec.md) is the contract.
It fixes the resource attributes, the metric names, types and labels, the label
rules, the log fields and Loki labels, and trace propagation. This ADR does not
restate them.

- `baukit-telemetry` sets the resource attributes from `ServiceIdentity`. The
  process part of `service.name` comes from `ProcessKind` and the environment
  from `DeploymentEnvironment`, both enums in `baukit-core`, so a value outside
  the spec does not compile. The collector adds Kubernetes attributes. Apps do
  not.
- The crate named in each spec section owns its metrics and records them once:
  `baukit-http` for HTTP server metrics, `baukit-ops` for the database pool,
  `baukit-jobs` for workers, `baukit-mcp` for `/mcp`. Products do not emit
  these names themselves.
- Prometheus exposition is on the operations listener at `/metrics`.
- Product metrics carry the product name as a prefix and follow the same label
  rules. A product lists them in its own allowlist for the lint.
- A change to the spec is a breaking change for `baukit-telemetry` and
  `baukit-http` and needs a migration note.

## Enforced by

- The `metric-names` gate step runs
  `deploy/observability/lint/check-metric-names.py` on `deploy/observability`
  and on `deploy/observability-platform` with its `metrics.txt` allowlist. It
  fails on a metric not in `SPEC_METRICS` or the allowlist, the plural
  `http_requests_duration_seconds`, a status-class label value such as `"5xx"`,
  `service_name` in a Loki selector, and `job` instead of `job_kind` in a worker
  aggregation. `SPEC_METRICS` is a copy of the spec's section 2 names. Nothing
  checks that the copy matches the spec.
- The `observability-tests` step runs
  `deploy/observability/lint/test_check_metric_names.py`, which tests those
  rejections.
- `baukit_test::assert_metrics_conformance` checks a process's exposition for
  the required names and exact label sets and rejects forbidden names and
  unbounded label shapes. The generated backend calls it in
  `templates/backend/backend/tests/conformance.rs`,
  `health_and_metrics_conform_to_baukit`, which the fixture steps run.
- `rust/crates/baukit-telemetry/src/lib.rs`,
  `resource_contains_exact_contract_attributes`, checks the attribute keys and
  values.

## Consequences

One dashboard, alert and recording rule set covers every product. Products
give up naming their own HTTP, pool and worker metrics, and products that
predate the spec had to rename metrics and Loki labels. The label rules ban
user identifiers and raw paths, so per-user questions go to logs or traces.

## History

- 2026-08-08, `9e525371`: the telemetry specification was written with the
  first platform specifications. It resolved the naming differences between
  the first products.
- 2026-08-09, `a5dd1acb`: `testing` became an accepted
  `deployment.environment.name`.
