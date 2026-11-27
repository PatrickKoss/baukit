# Get a service onto dashboards and alerts

At the end of this page a baukit service exposes metrics on its ops listener,
Prometheus scrapes it, the shared dashboard and alert pack show it, every alert
links to a runbook, and `baukit check observability` fails the gate when a
dashboard references a metric that does not exist. The page is for product
developers adding metrics and for whoever installs the observability pack.

The metric names, labels and log fields are fixed in
[the telemetry specification](../platform/telemetry-spec.md). This page shows
how the pieces connect and does not restate that contract.

## Start telemetry and the ops listener

Every process builds a `baukit_telemetry::TelemetryBuilder` from its
`ServiceInfo`, then passes the Prometheus handle to the ops router. From
`examples/minimal-api/src/main.rs`:

```rust,ignore
let service = ServiceInfo::new(PRODUCT, ProcessKind::Api, build_info!(), config.environment);
let mut telemetry_builder = TelemetryBuilder::new(service.telemetry_identity().clone())
    .sampling_ratio(config.telemetry.trace_sampling_ratio)
    .log_format(config.telemetry.log_format);
if let Some(endpoint) = &config.telemetry.otlp_endpoint {
    telemetry_builder = telemetry_builder.otlp_endpoint(endpoint);
}
let telemetry = Arc::new(telemetry_builder.init()?);
```

The ops listener is separate from the API listener. It binds
`ops.bind_address` and `ops.port`, default `0.0.0.0:9090`, and serves
`/healthz`, `/readyz` and `/metrics` through `baukit-ops`. The public API
listener answers `/metrics` with 404. Keep the ops port off the ingress.

Telemetry config keys, read from `<PREFIX>__TELEMETRY__<KEY>`:

| Key | Default | Effect |
|---|---|---|
| `otlp_endpoint` | `OTEL_EXPORTER_OTLP_ENDPOINT` when set, else none | where traces go; none disables export |
| `trace_sampling_ratio` | `1.0` | 0.0 to 1.0 |
| `log_format` | `auto` | `auto` picks JSON when deployed and pretty locally; `json` or `pretty` forces one |
| `resource.service_name`, `resource.service_version`, `resource.service_commit`, `resource.product` | none | override the identity `ServiceInfo` derives |

The chart sets `OTEL_EXPORTER_OTLP_ENDPOINT` from `config.otlpEndpoint`.

`baukit-http` records `http_requests_total`, `http_request_duration_seconds`
and `http_requests_in_flight` once per request. `baukit-ops` adds the pool
gauges when the database feature is on, and the worker runtime adds the
`worker_*` metrics. You do not record these yourself.

## Add a product metric

Product metrics use the `metrics` crate macros `counter!`, `gauge!` and
`histogram!` with a literal name. The name starts with the product prefix,
counters end in `_total`, and gauges and histograms do not. Labels follow the
label rules in section 2.5 of the telemetry spec: no paths, user ids, emails,
tokens, error text or request ids, and every label's value set is known at build
time.

The generated `baukit-check.toml` already tells the check where to look:

```toml
[observability]
dashboards = ["deploy/observability/dashboards"]
rules = ["deploy/observability/rules"]
sources = ["backend/crates"]
metric-prefix = "<app_crate>_"
```

That block comes from `templates/common/baukit-check.toml`, rendered with the
product's crate name. With it, `baukit check observability` finds every literal
`counter!`, `gauge!` and `histogram!` name under `backend/crates`, and fails when
a product metric is missing from every dashboard, breaks the suffix rules, or
lacks the prefix.

## Chart it and alert on it

A generated backend ships its own pack:

- `deploy/observability/dashboards/api.json`, rendered from
  `templates/backend/deploy/observability/dashboards/api.json.jinja`
- `deploy/observability/rules/api.rules.yml`, with the `ApiHighErrorRatio` alert
- `docs/runbooks/api-high-error-ratio.md`, which that alert's `runbook_url`
  points to

Add a panel for each new metric to the dashboard JSON. Name datasources through
a variable declared in `templating` or `__inputs`, or list a fixed uid in
`datasource-uids`. Write rule expressions against the raw `status` code with
`status=~"5.."`. Status classes such as `"5xx"` are not stored labels, and the
check rejects them. So does the plural `http_requests_duration_seconds`,
`service_name` in a Loki selector, and `job` instead of `job_kind` on worker
metrics.

Every alert gets a `runbook_url` and a runbook file. The runbook says what the
alert means and what to check first.

## Run the checks

```sh
baukit check observability
```

The rule reads `[observability]` from `baukit-check.toml`. Without the table it
prints a note and passes. A metric counts as existing when it is in baukit's
catalog, in a recording rule, in a `metrics` allowlist file, or emitted under
`sources`. For dashboards over third-party metrics, add an
`[[observability.packs]]` entry with its own `metrics` lists and
`catalog = false`. Names allowed in one pack never leak into another. The full
rule list is in [`lint/README.md`](../../lint/README.md).

The generated CI workflow runs `baukit check` with the `observability` rule in
the same job as the other rules. It also has an `observability-lint` job that
runs `deploy/observability/lint/check-metric-names.py` from the matching baukit
tag, but only when `deploy/observability/product-metrics.txt` exists. That file
lists product metric names one per line, with an optional `histogram` marker.
See the rendered `docs/observability-lint.md` in your product for its format.
`promtool check rules` and `promtool test rules` run in the product gate and
need Docker.

Inside baukit itself, after touching metric names or anything under
`deploy/observability`:

```sh
python3 deploy/observability/lint/check-metric-names.py
```

A new baukit metric goes into the telemetry spec first, then into the
`SPEC_METRICS` list in that script.

## Install the shared pack

`deploy/observability` is a Helm chart with baukit's service dashboard,
recording rules and alerts. From `deploy/observability/README.md`:

```sh
helm upgrade --install baukit-observability deploy/observability \
  --namespace observability --create-namespace \
  --set runbookBaseUrl=https://github.com/PatrickKoss/baukit/blob/main/docs/runbooks/app
```

`runbookBaseUrl` is required. The chart replaces the `RUNBOOK_BASE_URL` token in
every alert, so a render without it fails. Point it at `docs/runbooks/app/` in
the baukit release you deploy. Each alert slug has a file there, for example
[`rate-limit-store-errors.md`](../runbooks/app/rate-limit-store-errors.md).

The pack emits a dashboard ConfigMap labelled `grafana_dashboard: "1"`, rule
ConfigMaps, and a `PrometheusRule` when the cluster serves
`monitoring.coreos.com/v1`. The Prometheus `ruleSelector` must admit
`app.kubernetes.io/part-of: baukit-observability`.

The shipped alerts cover error budget burn, p95 and p99 latency, scrape and
target absence, missing `build_info`, database pool saturation and acquire
timeouts, rate-limit store errors, MCP errors, failed privacy operations, worker
failure, retry, queue age and failed jobs, and egress error ratio, latency and
open circuits. The
thresholds and the 99.9% SLO they assume are described in the pack README.
Change them there, then run `promtool check rules`.

## Scrape the service

In the `baukit-app` chart, set `serviceMonitor.enabled: true`. The chart then
creates a ServiceMonitor for the ops port with `serviceMonitor.interval`, default
`30s`, and `serviceMonitor.scrapeTimeout`, default `10s`. Add
`serviceMonitor.labels` when your Prometheus selects monitors by label. The
chart NetworkPolicy already admits the pods matched by
`networkPolicy.prometheus` to the ops ports.

Target discovery attaches the `product`, `service`, `environment` and
`namespace` labels. They are platform labels, not metric labels your code sets.

## Verify a running product

`deploy/observability/verify/verify-observability.sh` checks a product you
already started. It takes the environment prefix as its only argument and reads
`<PREFIX>_VERIFY_*` settings. It asserts that public `/metrics` is 404, scrapes
every ops listener, searches the logs for forbidden values such as test secrets,
and resolves every dashboard and alert metric against the live scrapes.
`<PREFIX>_VERIFY_LOG_FILES` and `<PREFIX>_VERIFY_FORBIDDEN_VALUES_FILE` are
required. The pack README has the full invocation.

## Reference

- [Telemetry specification](../platform/telemetry-spec.md)
- [`deploy/observability/README.md`](../../deploy/observability/README.md)
- [`lint/README.md`](../../lint/README.md), the `observability` rule
- [`templates/common/docs/observability-lint.md`](../../templates/common/docs/observability-lint.md)
- [`templates/common/baukit-check.toml`](../../templates/common/baukit-check.toml)
- [`rust/crates/baukit-telemetry/README.md`](../../rust/crates/baukit-telemetry/README.md)
- [`rust/crates/baukit-ops/README.md`](../../rust/crates/baukit-ops/README.md)
- [`deploy/chart/baukit-app/README.md`](../../deploy/chart/baukit-app/README.md)
- [`examples/minimal-api/src/main.rs`](../../examples/minimal-api/src/main.rs)
- [App runbooks](../runbooks/app/)
- [ADR 0029: observability contract](../adr/0029-observability-contract.md)
- [ADR 0011: separate API and ops listeners](../adr/0011-separate-api-and-ops-listeners.md)
- [Control outbound HTTP and request rates](outbound-and-limits.md)
- [Deploying](deploying.md)
