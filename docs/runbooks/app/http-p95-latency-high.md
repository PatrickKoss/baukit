# BaukitHttpP95LatencyHigh

## Meaning

The 95th percentile of `http_request_duration_seconds` for one route of `service` has been above
0.5s for 10 minutes. The recording rule `baukit:http_request_duration_seconds:p95_5m` is kept per
`route`, so the alert names the slow route.

## Impact

One request in twenty on that route takes longer than half a second. Users see slow screens; no
errors yet.

## First checks

Slowest routes:

```promql
topk(10, baukit:http_request_duration_seconds:p95_5m{service="<service>", environment="<environment>"})
```

Traffic on the route, to tell load from a regression:

```promql
sum by (route) (baukit:http_requests:rate5m{service="<service>", environment="<environment>"})
```

Database wait time, a frequent cause:

```promql
histogram_quantile(0.95, sum by (le) (rate(db_pool_acquire_duration_seconds_bucket{service="<service>", environment="<environment>"}[5m])))
```

CPU throttling of the API pods:

```sh
kubectl -n <namespace> top pods -l baukit.dev/process=api,app.kubernetes.io/part-of=<product>
```

## Fix

- Pool wait is high: see `BaukitDatabasePoolSaturated`.
- Traffic grew: raise replicas or the HPA maximum in the product overlay.
- A release made the route slower: roll back and profile the query or handler.
- A slow upstream call: add or tighten its timeout in the egress client.

## Escalation

Notify the product team. Escalate if `BaukitHttpP99LatencyHigh` also fires.
