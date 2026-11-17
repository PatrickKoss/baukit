# BaukitServiceTargetsAbsent

## Meaning

Prometheus has no `up` series with both a `product` and a `service` label, for 5 minutes. No product target is
being discovered. This is a discovery problem, not one broken pod.

## Impact

All application alerts are silent because they have no data. An outage would go unnoticed.

## First checks

Do any targets exist?

```promql
count by (job) (up)
```

ServiceMonitors from the product chart, and the Prometheus selector that should pick them up:

```sh
kubectl get servicemonitors -A -l app.kubernetes.io/component=api
kubectl -n observability get prometheus -o jsonpath='{.items[0].spec.serviceMonitorSelector}{"\n"}{.items[0].spec.serviceMonitorNamespaceSelector}{"\n"}'
```

The relabelings that add `service` (the chart sets `product`, `service` and `environment`):

```sh
kubectl -n <namespace> get servicemonitor <release>-api -o yaml | grep -A2 targetLabel
```

Operator errors:

```sh
kubectl -n observability logs deploy/kube-prometheus-stack-operator --tail=100
```

## Fix

- No ServiceMonitor: set `serviceMonitor.enabled: true` in the product's chart values.
- ServiceMonitor exists but is not selected: align its labels with `serviceMonitorSelector`, or
  label the product namespace for `serviceMonitorNamespaceSelector`.
- The operator cannot reconcile: fix the error it logs. A missing CRD after an upgrade is common.

## Escalation

Platform on-call. This hides every other application alert, so treat it as critical even at night.
