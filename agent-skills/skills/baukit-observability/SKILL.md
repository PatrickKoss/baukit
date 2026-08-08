---
name: baukit-observability
description: Connect a Baukit product to the shared Grafana dashboard, Prometheus recording rules, and alert rules while enforcing the telemetry metric, label, log, and trace contracts. Use when onboarding a service to shared monitoring, changing dashboards or alerts, or adding telemetry instrumentation.
---

# Wire shared observability

Use the observability pack from the same Baukit release train as the product. Never invent metric names or labels.

## Import the shared pack

1. Read and follow `<baukit-repo>/deploy/observability/README.md` as the import runbook; do not reproduce its assets by hand.
2. Provision or import `dashboards/baukit-service-overview.json` and bind its Prometheus and Loki datasource variables.
3. Mount both `recording-rules/*.rules.yml` and `alerts/*.rules.yml` in Prometheus. Load recording rules before interpreting dependent alerts.
4. Review the product SLO and all supplied thresholds together, replace placeholder runbook URLs, and configure the receiver to detect absence of `BaukitDeadMansSwitch` before production routing.

## Enforce telemetry conformance

Read `<baukit-repo>/docs/platform/telemetry-spec.md` before changing instrumentation. Use its exact section 2 metric names, types, labels, and buckets. In particular:

- Keep HTTP metrics owned and recorded exactly once by `baukit-http`; route labels are matched templates, status labels are raw numeric strings, and method/label values are bounded.
- Keep Loki labels limited to `service`, `environment`, `namespace`, and `level`; trace IDs, request IDs, product data, and sensitive values remain structured fields, not labels.
- Let the collector add Kubernetes resource attributes. Do not add unbounded user, URL, payload, token, email, error-message, trace, or request identifiers to metric labels.
- Treat a genuinely new shared metric as a contract change: update the telemetry specification and the observability linter in the Baukit release train before using it. Product-owned domain metrics still use a product prefix and bounded labels.

Keep or extend the generated backend conformance test so it scrapes metrics and calls `baukit_test::assert_metrics_conformance(..., true)`. Run it from the product root:

```sh
(cd backend && cargo test --test conformance)
```

## Lint and validate the pack

Run in the matching Baukit checkout after any dashboard, recording-rule, alert, or telemetry-contract change:

```sh
python3 deploy/observability/lint/check-metric-names.py
promtool check rules deploy/observability/recording-rules/*.rules.yml
promtool check rules deploy/observability/alerts/*.rules.yml
```

The Baukit linter rejects unknown or pluralized HTTP metrics, `service_name` Loki selectors, and stored status-class labels. Also run the product backend tests and validate the imported dashboard and rules in staging before enabling notifications.
