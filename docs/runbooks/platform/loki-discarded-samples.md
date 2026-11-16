# BaukitLokiDiscardedSamples

## Meaning

Loki rejected log lines for 10 minutes. The `reason` label says why: `rate_limited`,
`per_stream_rate_limit`, `line_too_long`, `greater_than_max_sample_age` and others.

## Impact

The rejected lines are gone. Falco events, audit trails and product logs may be missing when someone
needs them.

## First checks

```bash
kubectl -n observability logs -l app.kubernetes.io/name=loki --since=15m | grep -i discard
kubectl -n observability get configmap -l app.kubernetes.io/name=loki -o yaml | grep -A20 limits_config
```

```promql
sum by (reason, tenant) (rate(loki_discarded_samples_total[5m]))
topk(10, sum by (namespace) (rate(loki_distributor_lines_received_total[5m])))
```

## Fix

- Rate limits: find the noisy namespace and fix its log volume; raise `ingestion_rate_mb` only after
  checking memory headroom.
- `greater_than_max_sample_age`: Alloy is replaying an old backlog; it stops by itself.
- `line_too_long`: a workload logs huge lines; fix it.

## Escalation

Escalate to the platform owner if discards continue for an hour.
