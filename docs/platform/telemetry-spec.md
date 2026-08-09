# Telemetry specification

**Status:** Draft for adoption (Phase 0 deliverable)
**Home:** moves to `baukit/docs/telemetry-spec.md` when the baukit repository is created.
**Decisions behind it:** [platform analysis, section 17](../shared-application-platform-analysis.md#17-decision-log).

All backends built on baukit MUST conform. Conformance is enforced by `baukit-test` fixtures and a dashboard lint job in CI, not by review taste.

## 1. Resource attributes

Every process sets, via `baukit-telemetry`:

| Attribute | Example | Rule |
|---|---|---|
| `service.name` | `fitness-tracker-api` | `<product>-<process>`; process is one of `api`, `worker`, `migrate`, `seed` |
| `service.version` | `1.4.2` | Cargo package version |
| `service.commit` (custom) | `a1b2c3d` | short git SHA, injected at build time |
| `deployment.environment.name` | `production` | one of `local`, `testing`, `staging`, `production` |
| `product` (custom) | `fitness-tracker` | product identity, stable across all its processes |

Kubernetes attributes (`k8s.namespace.name`, workload, pod, cluster) are added by the collector (Alloy) at collection time, never by the application.

## 2. Metrics

Prometheus exposition on the operations listener at `/metrics`. The naming divergences found in section 8.1 of the analysis are resolved as follows.

### 2.1 HTTP server (owned by `baukit-http`, recorded exactly once)

| Metric | Type | Labels |
|---|---|---|
| `http_requests_total` | counter | `method`, `route`, `status` |
| `http_request_duration_seconds` | histogram | `method`, `route`, `status` |
| `http_requests_in_flight` | gauge | `method`, `route` |

Rules:

- The duration metric name is **singular** (`http_request_duration_seconds`). Fitness Tracker migrates from the plural name.
- `status` is the raw numeric status code as a string (`"200"`, `"404"`). Status classes are derived in queries and recording rules, never stored as labels. Solo Leveling migrates from status classes.
- `route` is always the matched route template (`/users/{id}`), never the raw path. Requests that match no route use `route="unmatched"`.
- `method` is uppercase and bounded to known HTTP methods; anything else becomes `OTHER`.
- Histogram buckets (seconds): `0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10`.
- Application-prefixed HTTP metric names (for example `sl_http_*`) are forbidden. Solo Leveling migrates.

### 2.2 Build info

Every process emits `build_info{version, commit, rust_version} = 1` (gauge).

### 2.3 Database pool (owned by `baukit-ops` when the database feature is enabled)

| Metric | Type |
|---|---|
| `db_pool_connections_max` | gauge |
| `db_pool_connections_idle` | gauge |
| `db_pool_connections_in_use` | gauge |
| `db_pool_acquire_duration_seconds` | histogram |
| `db_pool_acquire_timeouts_total` | counter |

### 2.4 Workers

| Metric | Type | Labels |
|---|---|---|
| `worker_job_runs_total` | counter | `job`, `outcome` (`success`, `failure`, `retry`) |
| `worker_job_duration_seconds` | histogram | `job` |
| `worker_queue_oldest_age_seconds` | gauge | `queue` |

`job` and `queue` values are static identifiers defined in code, never derived from payload data.

### 2.5 Label rules

Forbidden as label values anywhere: raw URL paths, user identifiers, email addresses, tokens, error message text, trace IDs, request IDs, and provider payload fields. Every label's value set must be bounded and known at build time.

Domain metrics are product-owned, use the product name as prefix (for example `fittrack_sync_conflicts_total`), and follow the same label rules.

## 3. Logs

- JSON to stdout in deployed environments; human-readable format in local development. The switch follows `deployment.environment.name`.
- Required fields: `timestamp` (RFC 3339), `level`, `message`, `target`, plus `trace_id`/`span_id` when inside a span and `request_id` when inside a request.
- Loki labels are bounded to: `service`, `environment`, `namespace`, `level`. The label is **`service`**, not `service_name`; Fitness Tracker migrates. Everything else, including `trace_id` and `request_id`, stays in the log body as fields.
- Scrubbed by default: `Authorization` headers, cookies, tokens, passwords, email addresses, request/response bodies, and provider payloads.

## 4. Traces

- OTLP export to the collector endpoint taken from `OTEL_EXPORTER_OTLP_ENDPOINT`.
- W3C Trace Context propagation, inbound and outbound.
- HTTP server spans are named `{METHOD} {route template}`.
- Explicit flush on shutdown, completed before the drain deadline.
- Sampling: 100% while traffic is low. Introduce tail-based, error/latency-biased sampling in the collector (not in applications) only when volume forces it.

## 5. Retention budgets (starting points)

Metrics 30 days, logs 14 days, traces 7 days. Adjust from observed cost and incident value, per section 8.5 of the analysis.

## 6. Conformance

- `baukit-test` ships a conformance test: boot the service, scrape `/metrics`, assert the section 2 names and labels exist, and assert no forbidden names (plural duration, app-prefixed HTTP metrics) appear.
- Dashboards, recording rules, and alerts in `deploy/observability/` are linted in CI against the names in this file.
- Changes to this specification are SemVer-relevant for `baukit-telemetry` and `baukit-http` and require a migration note in the release.

## 7. Migration checklist for the existing apps

| App | Required changes |
|---|---|
| Fitness Tracker | duration metric plural → singular; log label `service_name` → `service` |
| OpenDialog | remove duplicate HTTP recording (custom RED middleware plus `axum-prometheus`); keep exactly one recorder |
| Solo Leveling | drop app-prefixed HTTP metric names; status class → raw status code |
