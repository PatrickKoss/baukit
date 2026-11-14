# BaukitPrometheusStorageNearRetentionSize

## Meaning

Prometheus uses more than 90% of its `retentionSize`. It deletes the oldest blocks to stay under it.

## Impact

History is shorter than the 30-day `retention`. Dashboards and SLO windows that reach back further show
gaps.

## First checks

```bash
kubectl -n observability get prometheus -o jsonpath='{.items[*].spec.retentionSize}'
kubectl -n observability get pvc
```

```promql
prometheus_tsdb_storage_blocks_bytes / prometheus_tsdb_retention_limit_bytes
topk(10, count by (__name__) ({__name__=~".+"}))
prometheus_tsdb_head_series
```

## Fix

- Cardinality grew: the `topk` query names the metric; drop high-cardinality labels with a
  `metricRelabelings` rule on its monitor.
- Real growth: raise `retentionSize` and the PVC size together in Git.

## Escalation

Escalate to the platform owner if retention drops below 7 days.
