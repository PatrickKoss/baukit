# Baukit observability pack

This directory contains the portable application dashboard, Prometheus recording rules, and alert rules for the metric and log contract in [`docs/platform/telemetry-spec.md`](../../docs/platform/telemetry-spec.md). Target discovery must attach the bounded `product`, `service`, `environment`, and `namespace` labels; these are platform labels, not additional application metric dimensions.

## Grafana

Provision `dashboards/baukit-service-overview.json` with Grafana's file dashboard provider, or import it through the Grafana UI. Select Prometheus and Loki through the dashboard's `${DS_PROMETHEUS}` and `${DS_LOKI}` datasource variables. The Loki panel uses only the allowed `service` and `environment` labels; `product` remains a Prometheus-only filter because it is not an allowed Loki label.

A minimal provisioning layout is:

```yaml
apiVersion: 1
providers:
  - name: baukit
    type: file
    options:
      path: /var/lib/grafana/dashboards/baukit
```

Mount the dashboard JSON below that path. Datasource UIDs are deliberately not fixed, so the same dashboard can be imported in local, staging, and production Grafana instances.

## Prometheus

Mount both rule files and include them in `prometheus.yml`:

```yaml
rule_files:
  - /etc/prometheus/rules/baukit-recording/*.rules.yml
  - /etc/prometheus/rules/baukit-alerts/*.rules.yml
```

Load recording rules before evaluating alerts. Prometheus evaluates groups independently, so allow at least one recording interval after deployment before interpreting alert state.

## SLO and threshold tuning

The shipped availability template assumes a 99.9% SLO: the burn-rate recording rules divide the 5xx ratio by a `0.001` error budget. The fast pair uses 5m/1h windows at `14.4x`; the sustained pair uses 30m/6h windows at `6x`. A product adopting a different SLO must change the four divisors and review both alert thresholds together.

The default HTTP latency thresholds are p95 > 0.5s and p99 > 1s for ten minutes. Database saturation defaults to 90%, worker failures to 10%, retries to one per second, and oldest queue age to five minutes. Treat these as product templates and test changes with `promtool check rules`.

Replace every `https://runbooks.example.invalid/...` annotation before routing alerts to production. The always-firing `BaukitDeadMansSwitch` must be monitored for absence by the notification receiver.

## Lint contract

Run:

```sh
python3 deploy/observability/lint/check-metric-names.py
```

The standard-library-only linter reads dashboard expressions and Prometheus rule files. It rejects metric names outside telemetry spec section 2 (apart from Prometheus's `up` and locally defined recording rules), the forbidden plural HTTP duration name, Loki selectors using `service_name`, and stored status-class matchers such as `status=~"2xx"`. CI runs the same command; adding a metric requires updating the telemetry specification first and then its single `SPEC_METRICS` list in the linter.
