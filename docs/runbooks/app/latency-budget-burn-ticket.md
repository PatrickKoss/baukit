# BaukitLatencyBudgetBurnTicket

## Meaning

More than 1% of one service's requests took longer than 0.5s in both the 1d and the 3d window, for an
hour, with at least 100 requests in the 3d window. The latency SLO is 99% of requests under 0.5s, so
the 30-day latency budget runs out before the month ends. The target is the `le="0.5"` bucket of
`http_request_duration_seconds`; products with another target change the bucket and the `0.01`
divisor in the recording rules together.

## Impact

Some users wait noticeably on a share of their requests. Nothing fails, so error alerts stay quiet.

## First checks

Share of requests under the target per route, lowest first:

```promql
bottomk(5, baukit:http_requests_under_target:ratio5m{service="<service>", environment="<environment>"})
```

Over the whole ticket window:

```promql
sum by (route) (increase(http_request_duration_seconds_bucket{service="<service>", environment="<environment>", le="0.5"}[3d]))
/
sum by (route) (increase(http_request_duration_seconds_count{service="<service>", environment="<environment>"}[3d]))
```

Whether the time goes to waiting for a database connection:

```promql
histogram_quantile(0.95, sum by (le) (rate(db_pool_acquire_duration_seconds_bucket{service="<service>", environment="<environment>"}[1h])))
```

## Fix

- One slow route: look at its traces in Tempo, add the missing index or cut the work per request.
- Every route slow and acquire waits high: follow `BaukitDatabaseAcquireLatencyHigh`.
- Slow only at peak: scale the API (`kubectl -n <namespace> get hpa`).

## Escalation

Ticket for the product team, handled within the week. No page.
