# Telemetry specification

**Status:** Adopted.
**Home:** this repository.

All backends built on baukit MUST conform. Conformance is enforced by `baukit-test` fixtures and a dashboard lint job in CI, not by review taste.

## 1. Resource attributes

Every process sets, via `baukit-telemetry`:

| Attribute | Example | Rule |
|---|---|---|
| `service.name` | `example-api` | `<product>-<process>`; process is one of `api`, `worker`, `migrate`, `seed`, `agent` |
| `service.version` | `1.4.2` | Cargo package version |
| `service.commit` (custom) | `a1b2c3d` | short git SHA, injected at build time |
| `deployment.environment.name` | `production` | one of `local`, `testing`, `staging`, `production` |
| `product` (custom) | `example` | product identity, stable across all its processes |

Kubernetes attributes (`k8s.namespace.name`, workload, pod, cluster) are added by the collector (Alloy) at collection time, never by the application.

## 2. Metrics

Prometheus exposition on the operations listener at `/metrics`. The naming divergences found in section 8.1 of the analysis are resolved as follows.

### 2.1 HTTP server (owned by `baukit-http`, recorded exactly once)

| Metric | Type | Labels |
|---|---|---|
| `http_requests_total` | counter | `method`, `route`, `status` |
| `http_request_duration_seconds` | histogram | `method`, `route`, `status` |
| `http_requests_in_flight` | gauge | `method`, `route` |
| `http_requests_shed_total` | counter | `reason` (`concurrency`) |

Rules:

- The duration metric name is **singular** (`http_request_duration_seconds`).
- `status` is the raw numeric status code as a string (`"200"`, `"404"`). Status classes are derived in queries and recording rules, never stored as labels.
- `route` is always the matched route template (`/users/{id}`), never the raw path. Requests that match no route use `route="unmatched"`.
- `method` is uppercase and bounded to known HTTP methods; anything else becomes `OTHER`.
- Histogram buckets (seconds): `0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10, 30`. The `30` bucket matches the default request timeout, so timed-out requests land below `+Inf`.
- `http_requests_in_flight` counts requests that hold a concurrency permit. Requests waiting for one are not counted.
- `http_requests_shed_total` counts requests that got no concurrency permit within the queue timeout and were answered with `503` and `Retry-After: 1`. Shed requests also appear in `http_requests_total` with `status="503"`.
- Application-prefixed HTTP metric names (for example `sl_http_*`) are forbidden.

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
| `worker_job_runs_total` | counter | `job_kind`, `outcome` (`success`, `failure`, `retry`, `cancelled`) |
| `worker_job_duration_seconds` | histogram | `job_kind` |
| `worker_queue_oldest_age_seconds` | gauge | `queue` |
| `worker_queue_depth` | gauge | `queue`, `status` (`pending`, `running`, `failed`) |
| `worker_lease_expired_total` | counter | `queue` |
| `worker_job_deferrals_total` | counter | `job_kind` |

`job_kind` and `queue` values are static identifiers defined in code, never derived from payload data.

`worker_queue_oldest_age_seconds` is the time since `run_after` of the oldest due pending row of the runner's job types; rows scheduled for later count as zero. Runners sample it and `worker_queue_depth` every 15 s by default.

### 2.5 Label rules

Forbidden as label values anywhere: raw URL paths, user identifiers, email addresses, tokens, error message text, trace IDs, request IDs, and provider payload fields. Every label's value set must be bounded and known at build time.

Domain metrics are product-owned, use the product name as prefix (for example `fittrack_sync_conflicts_total`), and follow the same label rules.

### 2.6 Remote MCP (owned by `baukit-mcp`)

| Metric | Type | Labels |
|---|---|---|
| `mcp_requests_total` | counter | `status` |
| `mcp_request_duration_seconds` | histogram | none |
| `mcp_authentication_policy_decisions_total` | counter | `outcome` (`allowed`, `inactive`, `insufficient_scope`, `unavailable`, `limited`) |
| `mcp_authentication_policy_duration_seconds` | histogram | none |
| `mcp_introspection_cache_requests_total` | counter | `outcome` (`hit`, `miss`) |

These metrics cover authorization and execution on `/mcp`. Status uses exact
numeric HTTP codes. Token values, subjects, and tool arguments are never labels.
MCP quota decisions use `http_rate_limit_decisions_total` with scope `mcp` and
outcomes `allowed` or `limited`.

### 2.7 Process and runtime (owned by `baukit-telemetry`)

| Metric | Type | Labels |
|---|---|---|
| `process_cpu_seconds_total` | gauge | none |
| `process_resident_memory_bytes` | gauge | none |
| `process_open_fds` | gauge | none |
| `process_max_fds` | gauge | none |
| `process_start_time_seconds` | gauge | none |
| `tokio_runtime_workers` | gauge | none |
| `tokio_runtime_alive_tasks` | gauge | none |
| `tokio_runtime_global_queue_depth` | gauge | none |
| `tokio_runtime_busy_seconds_total` | gauge | none |

`baukit-telemetry` reads the `process_*` values from `/proc/self` every 5 seconds, so they are missing on systems without `/proc`. CPU seconds and busy seconds are monotonic but exported as gauges because the `metrics` facade only has integer counters; `rate()` works on them as usual. `process_start_time_seconds` is the time telemetry was initialized. The `tokio_runtime_*` gauges use only metrics that stable Tokio exposes without `tokio_unstable`, and appear when `TelemetryBuilder::init` runs inside a Tokio runtime.

### 2.8 Identity key caches (owned by `baukit-auth` and `baukit-suite`)

| Metric | Type | Labels |
|---|---|---|
| `auth_key_cache_requests_total` | counter | `source` (`oidc`, `ticket`), `result` (`fresh`, `stale`, `fetched`, `failed`) |

Each key lookup counts once: `fresh` and `stale` were served from the cache, `fetched` came from a fetch made by that lookup, and `failed` means the lookup got no key set or its own fetch failed while a stale set was served. A background refresh of a stale set also counts once, as `fetched` or `failed`. A rising `stale` or `failed` rate means the issuer's JWKS endpoint is unreachable while verification still runs on cached keys.

### 2.9 Outbound HTTP (owned by `baukit-egress`)

| Metric | Type | Labels |
|---|---|---|
| `egress_requests_total` | counter | `destination`, `status`, `outcome` (`success`, `unavailable`, `timeout`, `rate_limited`, `revoked`, `permanent`, `circuit_open`, `saturated`) |
| `egress_request_duration_seconds` | histogram | `destination` |
| `egress_circuit_state` | gauge | `destination` (`0` closed, `1` half-open, `2` open) |
| `egress_circuit_transitions_total` | counter | `destination`, `to` (`closed`, `half_open`, `open`) |

`destination` is the name passed to `GuardedClient::new` (`oidc`, `idp`, `hub`, `suite-peer`, `expo`, `keycloak-admin`, `mcp-introspection`, or a product provider name), never a host. `status` is the numeric response status, or `none` when no response arrived. Each call to `execute` or `execute_stream` adds one `egress_requests_total` sample, including calls refused by an open circuit or a full in-flight cap. `egress_request_duration_seconds` measures the time to the response headers of requests that were sent and uses the HTTP buckets from 2.1. `egress_circuit_state` is the worst state across the `(scheme, host, port)` breakers of one destination. A breaker reports open until the next call moves it to half-open, so a destination nobody calls keeps reporting open.

### 2.10 Push (owned by `baukit-push`)

| Metric | Type | Labels |
|---|---|---|
| `push_notifications_total` | counter | `outcome` (`delivered`, `accepted`, `token_dead`, `rejected`, `failed`) |

`ExpoPushSender::send` adds one sample per notification per call. `accepted` is with the provider and waiting for its receipt, `token_dead` is a `DeviceNotRegistered` rejection, `rejected` is any other provider refusal, and `failed` counts the notifications left unsent by a failed provider request. Receipts settled later by `poll_pending_receipts` are not counted. Tokens, owners and device ids are never labels.

### 2.11 Suite events (owned by `baukit-suite`)

| Metric | Type | Labels |
|---|---|---|
| `suite_deliveries_total` | counter | `outcome` (`delivered`, `retry`, `deferred`, `rejected`, `unauthorized`, `revoked`, `disabled`) |
| `suite_events_ingested_total` | counter | `outcome` (`granted`, `no_rule`, `capped`, `duplicate`, `rejected`, `unauthorized`) |
| `suite_emission_skipped_total` | counter | `reason` (`invalid`, `timeout`, `storage`) |
| `suite_cleanup_failures_total` | counter | none |

The names are fixed. The product comes from the scrape labels (`service`, `environment`), not from a name prefix. `suite_deliveries_total` counts each peer delivery attempt, and `deferred` means an open egress breaker held it back. `suite_emission_skipped_total` counts live events that never reached the jobs outbox. `suite_cleanup_failures_total` counts hourly cleanup runs that hit a storage error.

## 3. Logs

- JSON to stdout in deployed environments; human-readable format in local development. The switch follows `deployment.environment.name`.
- Required fields: `timestamp` (RFC 3339), `level`, `message`, `target`, plus `trace_id`/`span_id` when inside a span and `request_id` when inside a request.
- Loki labels are bounded to: `service`, `environment`, `namespace`, `level`. The label is **`service`**, not `service_name`. Everything else, including `trace_id` and `request_id`, stays in the log body as fields.
- Scrubbed by default: `Authorization` headers, cookies, tokens, passwords, email addresses, request/response bodies, and provider payloads.

## 4. Traces

- OTLP export to the collector endpoint taken from `OTEL_EXPORTER_OTLP_ENDPOINT`.
- W3C Trace Context propagation, inbound and outbound.
- HTTP server spans are named `{METHOD} {route template}`.
- Explicit flush on shutdown, completed before the drain deadline.
- Sampling: 100% while traffic is low. Introduce tail-based, error/latency-biased sampling in the collector (not in applications) only when volume forces it.

## 5. Retention budgets (starting points)

Metrics 30 days, logs 14 days, traces 7 days. Adjust them from observed cost and incident value.

## 6. Conformance

- `baukit-test` ships a conformance test: boot the service, scrape `/metrics`, assert the section 2 names and labels exist, and assert no forbidden names (plural duration, app-prefixed HTTP metrics) appear.
- Dashboards, recording rules, and alerts in `deploy/observability/` are linted in CI against the names in this file.
- Changes to this specification are SemVer-relevant for `baukit-telemetry` and `baukit-http` and require a migration note in the release.
