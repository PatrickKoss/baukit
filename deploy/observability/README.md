# Baukit observability pack

This directory contains the portable application dashboard, Prometheus recording rules, alert rules, Kubernetes provisioning, and a live verification harness for the metric and log contract in [`docs/platform/telemetry-spec.md`](../../docs/platform/telemetry-spec.md). Target discovery must attach the bounded `product`, `service`, `environment`, and `namespace` labels; these are platform labels, not additional application metric dimensions.

## Kubernetes provisioning

Install the pack into the observability namespace:

```sh
helm upgrade --install baukit-observability deploy/observability \
  --namespace observability --create-namespace
```

The chart emits separate dashboard, recording-rule, and alert-rule ConfigMaps. The dashboard has `grafana_dashboard: "1"`, the default discovery label used by Grafana dashboard sidecars. Labels and annotations can be extended through `dashboardConfigMap` and `ruleConfigMaps` values.

When the cluster advertises `monitoring.coreos.com/v1/PrometheusRule`, the chart also combines the exact shipped recording and alert groups into a `PrometheusRule`. Ensure the Prometheus instance's `spec.ruleSelector` admits `app.kubernetes.io/part-of: baukit-observability`; no product needs to copy rule content into a manifest. Set `prometheusRule.enabled=false` if another rule loader consumes the ConfigMaps. The application chart's `serviceMonitor.enabled=true` supplies scrape discovery separately, because scrape targets belong to each product release.

Plain Prometheus installations can mount the `*-recording-rules` and `*-alert-rules` ConfigMaps and use the paths below. The ConfigMaps keep their source filenames stable.

## Grafana

Outside Kubernetes, provision `dashboards/baukit-service-overview.json` with Grafana's file dashboard provider, or import it through the Grafana UI. Select Prometheus and Loki through the dashboard's `${DS_PROMETHEUS}` and `${DS_LOKI}` datasource variables. The Loki panel uses only the allowed `service` and `environment` labels; `product` remains a Prometheus-only filter because it is not an allowed Loki label.

A minimal provisioning layout is:

```yaml
apiVersion: 1
providers:
  - name: baukit
    type: file
    options:
      path: /var/lib/grafana/dashboards/baukit
```

Mount the dashboard JSON below that path. Datasource UIDs are deliberately not fixed, so the same dashboard can be imported in local, testing, staging, and production Grafana instances.

## Prometheus

Mount both rule files and include them in `prometheus.yml`:

```yaml
rule_files:
  - /etc/prometheus/rules/baukit-recording/*.rules.yml
  - /etc/prometheus/rules/baukit-alerts/*.rules.yml
```

Load recording rules before evaluating alerts. Prometheus evaluates groups independently, so allow at least one recording interval after deployment before interpreting alert state.

## SLO and threshold tuning

The shipped availability template assumes a 99.9% SLO: the burn-rate recording rules divide the 5xx ratio by a `0.001` error budget. Every ratio ends with `or on() vector(0)`, so an idle or all-success window renders as zero instead of disappearing. The fast pair uses 5m/1h windows at `14.4x`; the sustained pair uses 30m/6h windows at `6x`. A product adopting a different SLO must change the four divisors and review both alert thresholds together.

The default HTTP latency thresholds are p95 > 0.5s and p99 > 1s for ten minutes. Database saturation defaults to 90%, worker failures to 10%, retries to one per second, and oldest queue age to five minutes. Rate-limit store errors alert at critical severity when `outcome="error"` remains non-zero for ten minutes, because a default fail-open limiter may no longer enforce quotas. Treat these as product templates and test changes with `promtool check rules`.

Replace every `https://runbooks.example.invalid/...` annotation before routing alerts to production. The always-firing `BaukitDeadMansSwitch` must be monitored for absence by the notification receiver.

## Lint contract

Run:

```sh
python3 deploy/observability/lint/check-metric-names.py
```

The standard-library-only linter reads dashboard expressions and Prometheus rule files. It rejects metric names outside telemetry spec section 2 (apart from Prometheus's `up` and locally defined recording rules), the forbidden plural HTTP duration name, Loki selectors using `service_name`, and stored status-class matchers such as `status=~"2xx"`. CI runs the same command; adding a metric requires updating the telemetry specification first and then its single `SPEC_METRICS` list in the linter.

## Live verification harness

`verify/verify-observability.sh` checks a running product without embedding its build or startup logic. Its one argument is the product's normalized environment-variable prefix. Settings are then read as `<PREFIX>_VERIFY_*`, allowing the same script to be used unchanged by every product.

Required settings are `LOG_FILES` (colon-delimited runtime log paths) and `FORBIDDEN_VALUES_FILE` (one non-empty secret or sensitive literal per line). The API defaults to `http://127.0.0.1:18080` with ops on `:19090`; set `WORKER_OPS_URL` when a worker is present. `KNOWN_GAPS_FILE` is optional and contains one unresolved metric per line with `#` comments allowed. It is an exact allowlist: verification fails for both a new gap and a fixed gap that was not removed from the file.

```sh
MY_PRODUCT_VERIFY_API_PUBLIC_URL=http://127.0.0.1:18080 \
MY_PRODUCT_VERIFY_API_OPS_URL=http://127.0.0.1:19090 \
MY_PRODUCT_VERIFY_WORKER_OPS_URL=http://127.0.0.1:19091 \
MY_PRODUCT_VERIFY_LOG_FILES="$tmp/api.log:$tmp/worker.log" \
MY_PRODUCT_VERIFY_FORBIDDEN_VALUES_FILE="$tmp/forbidden-values" \
MY_PRODUCT_VERIFY_KNOWN_GAPS_FILE="$PWD/deploy/known-observability-gaps.txt" \
deploy/observability/verify/verify-observability.sh MY_PRODUCT
```

The harness lints expressions, checks all rule files with local `promtool` or a pinned Prometheus container, waits for health and readiness, asserts that public `/metrics` is exactly 404, scrapes every configured private ops listener, searches the supplied logs for every forbidden literal, and resolves all dashboard/alert metric references against the union of live scrapes and local recording rules. The caller owns product startup and teardown so Compose, host-process, and Kubernetes consumers can share the same verifier.

`verify/check-observability-metrics.py` can be called directly with repeated `--metrics process=path` arguments and repeated `--known-gap` or `--known-gap-file` options. `verify/known-gaps.example.txt` documents the allowlist format.
