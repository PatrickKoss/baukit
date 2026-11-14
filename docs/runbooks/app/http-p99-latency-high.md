# BaukitHttpP99LatencyHigh

## Meaning

The 99th percentile of `http_request_duration_seconds` for one route of `service` has been above 1s
for 10 minutes. This is critical: the tail is long enough that clients start to time out.

## Impact

One request in a hundred on that route takes over a second. Mobile clients with short timeouts
retry, which adds load and can push the p95 up next.

## First checks

Slowest routes at p99:

```promql
topk(10, baukit:http_request_duration_seconds:p99_5m{service="<service>", environment="<environment>"})
```

Is it every pod or one?

```promql
histogram_quantile(0.99, sum by (instance, le) (rate(http_request_duration_seconds_bucket{service="<service>", environment="<environment>"}[5m])))
```

Pool timeouts and restarts:

```promql
increase(db_pool_acquire_timeouts_total{service="<service>", environment="<environment>"}[10m])
```

```sh
kubectl -n <namespace> get pods -l baukit.dev/process=api,app.kubernetes.io/part-of=<product> \
  -o custom-columns=NAME:.metadata.name,RESTARTS:.status.containerStatuses[0].restartCount,NODE:.spec.nodeName
```

Warnings logged during slow requests:

```logql
{service="<service>", environment="<environment>", level=~"warn|error"} | json | line_format "{{.target}} {{.message}}"
```

## Fix

- One pod is slow: delete it so the Deployment replaces it, then check its node
  (`kubectl describe node <node>`) for pressure.
- All pods: work through the p95 runbook fixes. Database acquire timeouts come first.
- A lock or long transaction on the database: find it with
  `SELECT pid, now() - xact_start, query FROM pg_stat_activity ORDER BY 2 DESC LIMIT 10;`.

## Escalation

Page the product owner if it lasts 30 minutes or comes with 5xx errors. Escalate to the platform
on-call if the slow pods share a node.
