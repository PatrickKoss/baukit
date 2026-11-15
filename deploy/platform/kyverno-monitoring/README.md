# Kyverno monitoring

The parts of the Kyverno install that need the prometheus-operator CRDs, split out of `kyverno` so
Kyverno and its policies can come up before the observability stack.

- `kyverno-monitoring-values` turns on the ServiceMonitors of the four Kyverno controllers. The
  `kyverno` HelmRelease reads it through an optional `valuesFrom`, and its
  `reconcile.fluxcd.io/watch` label makes helm-controller upgrade the release when it appears.
- Policy Reporter chart `3.11.2`, with metrics and its ServiceMonitor on, UI, Kyverno plugin and
  Grafana dashboards off. It only reports, so nothing waits for it.

Order this base after `kyverno-policies` and the observability stack.
