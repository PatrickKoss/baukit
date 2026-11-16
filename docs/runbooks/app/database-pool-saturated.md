# BaukitDatabasePoolSaturated

## Meaning

More than 90% of the SQLx pool connections of one instance are in use
(`db_pool_connections_in_use / db_pool_connections_max`) for 10 minutes.

## Impact

New queries wait for a connection. Latency rises first, then acquires time out
(`BaukitDatabaseAcquireTimeouts`) and requests fail.

## First checks

Usage per pod:

```promql
sum by (instance) (db_pool_connections_in_use{service="<service>", environment="<environment>"})
/ sum by (instance) (db_pool_connections_max{service="<service>", environment="<environment>"})
```

Time spent waiting for a connection:

```promql
histogram_quantile(0.95, sum by (le) (rate(db_pool_acquire_duration_seconds_bucket{service="<service>", environment="<environment>"}[5m])))
```

Load versus the previous day:

```promql
sum(baukit:http_requests:rate5m{service="<service>", environment="<environment>"})
/ sum(baukit:http_requests:rate5m{service="<service>", environment="<environment>"} offset 1d)
```

Long-running queries on the database side:

```sh
kubectl -n <namespace> exec -it <cluster>-1 -- psql -c \
  "SELECT pid, state, now() - query_start AS age, left(query, 80) FROM pg_stat_activity WHERE datname = current_database() ORDER BY age DESC LIMIT 10;"
```

## Fix

- Slow or stuck queries hold connections: cancel them (`SELECT pg_cancel_backend(<pid>);`) and fix
  the query or add the missing index.
- Load grew: raise `<PREFIX>__DATABASE__MAX_CONNECTIONS` (default 10), keeping
  `replicas * max_connections` below the server's `max_connections`.
- A code path holds a connection across an outbound call: release it before the call.

## Escalation

Product team. Escalate to the database owner if the server is near its own connection limit.
