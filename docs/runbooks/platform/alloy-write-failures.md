# BaukitAlloyWriteFailures

## Meaning

An Alloy pod dropped log entries after its retries to the Loki gateway ran out.

## Impact

Logs from the node that pod runs on are missing in Loki for that window.

## First checks

```bash
kubectl -n observability logs <pod> --since=15m | grep -i 'loki.write'
kubectl -n observability get pods -l app.kubernetes.io/name=loki -o wide
```

```promql
sum by (pod, reason) (rate(loki_write_dropped_entries_total[5m]))
histogram_quantile(0.99, sum by (le, status_code) (rate(loki_write_request_duration_seconds_bucket[5m])))
```

## Fix

- Loki is down or slow: fix Loki first.
- 429 from Loki: see `loki-discarded-samples`.
- Network: the `observability` NetworkPolicy must allow Alloy to reach `loki-gateway`.

## Escalation

Escalate to the platform owner if logs are lost for more than 30 minutes.
