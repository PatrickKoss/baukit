# Watchdog

## Meaning

`Watchdog` comes from kube-prometheus-stack and always fires. Alertmanager routes it to the `heartbeat`
receiver every minute. The external heartbeat service alerts when those notifications stop. This page
is for that external alert, and for a `Watchdog` that shows up anywhere else.

## Impact

When the heartbeat stops, no platform alert reaches anyone. Prometheus, Alertmanager, the routing
Secret or the egress path to the heartbeat service is broken.

## First checks

```bash
kubectl -n observability get alertmanager,statefulset,pods -l app.kubernetes.io/name=alertmanager
kubectl -n observability describe pod alertmanager-kube-prometheus-stack-alertmanager-0 | grep -A5 Events
kubectl -n observability get secret alertmanager-routing
kubectl -n observability logs alertmanager-kube-prometheus-stack-alertmanager-0 -c alertmanager --since=30m
```

```promql
ALERTS{alertname="Watchdog"}
rate(alertmanager_notifications_failed_total{integration="webhook"}[5m])
```

## Fix

- Pod stuck in `ContainerCreating` with `secret "alertmanager-routing" not found`: the overlay did not
  provide the routing Secret. Add it (see
  `deploy/platform/observability-stack/alertmanager/alertmanager.example.yaml`).
- Secret present but the operator logs `config secret not found` or `key not found`: the key must be
  `alertmanager.yaml`.
- Notifications fail: check the `heartbeat` receiver URL and the Alertmanager egress NetworkPolicy.
- `Watchdog` not firing: Prometheus is down or does not select the kube-prometheus-stack rules.

## Escalation

Page the platform owner. While the heartbeat is down, watch the Grafana alert list by hand.
