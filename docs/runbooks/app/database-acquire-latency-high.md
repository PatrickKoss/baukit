# BaukitDatabaseAcquireLatencyHigh

## Meaning

The p95 time a request waited for a pooled database connection
(`db_pool_acquire_duration_seconds`) has been above 100ms for 10 minutes, on a pod acquiring at least
one connection every 10 seconds. Requests queue for the pool before they run any SQL.

## Impact

Every request that touches the database gets slower by the wait. If the wait reaches the
`acquire_timeout` (default 5s), `BaukitDatabaseAcquireTimeouts` fires and requests fail.

## First checks

Wait per pod and pool usage:

```promql
histogram_quantile(0.95, sum by (instance, le) (rate(db_pool_acquire_duration_seconds_bucket{service="<service>", environment="<environment>"}[5m])))
```

```promql
db_pool_connections_in_use{service="<service>", environment="<environment>"} / db_pool_connections_max{service="<service>", environment="<environment>"}
```

Slow queries that hold connections, from the database side:

```sql
SELECT pid, now() - xact_start AS age, state, left(query, 80)
FROM pg_stat_activity WHERE datname = current_database() AND state <> 'idle'
ORDER BY age DESC LIMIT 10;
```

Through the shared pooler, check PgBouncer's waiting clients with
`cnpg_pgbouncer_pools_cl_waiting` from the `postgres-cluster-pooler-rw` PodMonitor.

## Fix

- Pool full and queries slow: fix the slow query or the transaction held across an outbound call.
- Pool full and queries fast: raise `database.max_connections`, staying inside the tenant
  `connectionLimit` (see `deploy/platform/postgres-cluster/README.md`, "Connection budget").
- Pool not full: connections are slow to open. Check the database and the pooler health.

## Escalation

Product team during working hours. Page if `BaukitDatabaseAcquireTimeouts` fires too.
