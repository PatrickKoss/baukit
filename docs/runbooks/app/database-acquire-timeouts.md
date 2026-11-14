# BaukitDatabaseAcquireTimeouts

## Meaning

`db_pool_acquire_timeouts_total` grew in the last 10 minutes, and kept growing for 5 minutes.
A request waited the full `acquire_timeout` (default 5s) and got no connection.

## Impact

Each timeout is a failed request, usually a 5xx. This is an active outage for the affected paths.

## First checks

Timeouts per pod:

```promql
sum by (instance) (increase(db_pool_acquire_timeouts_total{service="<service>", environment="<environment>"}[10m]))
```

Is the pool full, or is the database unreachable?

```promql
db_pool_connections_in_use{service="<service>", environment="<environment>"}
```

If in-use is low while timeouts climb, the pool cannot open connections at all. Check the database:

```sh
kubectl -n <namespace> get cluster.postgresql.cnpg.io
kubectl -n <namespace> get pods -l cnpg.io/cluster=<cluster>
```

Connection errors in the logs:

```logql
{service="<service>", environment="<environment>", level="error"} | json | message=~"(?i).*(pool|connection|timed out).*"
```

## Fix

- Pool full: follow `BaukitDatabasePoolSaturated`.
- Database down or failing over: wait for the CNPG primary to come back, then confirm the
  timeouts stop. If the primary does not recover, follow the CNPG runbooks in `docs/runbooks/platform/`.
- Network policy or DNS change: check that the API pod can reach the `-rw` service on 5432.

## Escalation

Page the product owner. Page the platform on-call at the same time if the database cluster is not
healthy.
