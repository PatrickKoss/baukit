# BaukitFalcosidekickOutputErrors

## Meaning

Falcosidekick failed to forward Falco events to the output named by `destination` (`loki` or
`alertmanager`) for 10 minutes.

## Impact

With `loki` failing, the Falco dashboard's event panel and the LogQL queries in the other runbooks miss
events. With `alertmanager` failing, Critical and Warning events reach nobody's pager.
`BaukitFalcoCriticalEvent` still fires from Prometheus, so critical events are not lost entirely.

## First checks

```bash
kubectl -n falco logs deploy/falco-falcosidekick --tail=100
kubectl -n falco get networkpolicy
kubectl -n observability get svc loki-gateway kube-prometheus-stack-alertmanager
```

```promql
sum by (destination, status) (rate(falcosecurity_falcosidekick_outputs_total[5m]))
```

Test the path from the sidekick namespace:

```bash
kubectl -n falco run netcheck --rm -it --restart=Never --image=busybox:1.37 \
  --labels=app.kubernetes.io/name=falcosidekick -- \
  wget -qO- -T 5 http://kube-prometheus-stack-alertmanager.observability.svc.cluster.local:9093/-/ready
```

## Fix

- `connection refused` or timeouts: Loki's gateway or Alertmanager is down, or a NetworkPolicy blocks
  the path. The `allow-falcosidekick` policy in `falco` must allow egress, and the observability
  policies must allow ingress from `falco` pods labelled `app.kubernetes.io/name=falcosidekick`.
- HTTP 404 from Alertmanager: the endpoint is `/api/v1/alerts`, which Alertmanager 0.27 removed. The
  base sets `/api/v2/alerts`; check the overlay did not change it.
- HTTP 400 from Loki: an `extralabels` entry produced an invalid label. Check the values in
  `deploy/platform/falco/helm-release.yaml`.

## Escalation

If Alertmanager delivery stays broken for more than an hour, tell the platform owner: Falco events then
only surface through `BaukitFalcoCriticalEvent`.
