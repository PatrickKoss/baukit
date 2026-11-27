# BaukitTraefikServiceErrorBudgetBurnFast

## Meaning

More than 1.44% of the requests Traefik routed to one service returned 5xx in both the 5m and the 1h
window, with at least 100 requests in the last hour. That burns a 99.9% budget 14.4 times too fast.
Traefik counts its own 502 and 503 answers here, so this alert still fires when every pod of the
service is down and the app's own `BaukitHighErrorBudgetBurnFast` has no data. The `service` label is
`<namespace>-<name>-<port>@kubernetes`.

Traefik answers 503 for a service with no ready endpoints only when the provider keeps empty
services (`allowEmptyServices: true`). Without it Traefik drops the route, answers 404 and this alert
stays quiet.

## Impact

Users of that service see errors right now.

## First checks

```bash
kubectl -n <namespace> get pods,endpointslices -l kubernetes.io/service-name=<name>
kubectl -n <namespace> describe deploy <name>
```

```promql
sum by (code) (rate(traefik_service_requests_total{service="<service>"}[5m]))
```

If the app pack's fast burn alert is quiet at the same time, the requests never reach the app: no
ready pods, or a crash loop.

## Fix

- No ready endpoints: fix the crash or failing readiness, or roll back the release with Flux.
- Pods ready but 502/504: check the app's latency and the Traefik forwarding timeouts.
- 500/503 from the app: follow `BaukitHighErrorBudgetBurnFast`.

## Escalation

Page the product owner of the namespace. Page the platform owner if several services burn at once.
