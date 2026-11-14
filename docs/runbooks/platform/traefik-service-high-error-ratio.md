# BaukitTraefikServiceHighErrorRatio

## Meaning

More than 5% of the requests Traefik sent to one service returned 5xx for 10 minutes, with at least
one request every 10 seconds. The `service` label is Traefik's name, `<namespace>-<name>-<port>@kubernetes`.

## Impact

Users of that service see errors. 502 and 504 usually mean the backend is down or slow; 500 and 503
come from the application.

## First checks

```bash
kubectl -n <namespace> get pods,endpoints <name>
kubectl -n <namespace> logs deploy/<name> --since=15m
kubectl -n traefik logs deploy/traefik --since=15m | grep '<service>'
```

```promql
sum by (code) (rate(traefik_service_requests_total{service="<service>"}[5m]))
```

```logql
{namespace="traefik"} | json | ServiceName="<service>" | DownstreamStatus >= 500
```

## Fix

- 502/503 with no ready endpoints: the pods crash or fail readiness; fix the rollout or roll back with
  Flagger or Flux.
- 504: the backend is slower than the Traefik timeout; check its latency and database.
- 500 from the app: follow the product's own runbook for its error alert.

## Escalation

Escalate to the product owner of the namespace. Escalate to the platform owner if every service shows
errors, which points at Traefik itself.
