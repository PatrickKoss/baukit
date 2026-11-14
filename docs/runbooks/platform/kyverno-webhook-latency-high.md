# BaukitKyvernoWebhookLatencyHigh

## Meaning

The 99th percentile of Kyverno admission reviews has been above 1 second for 10 minutes.

## Impact

Every create and update in the cluster waits for Kyverno. At the 10-second webhook timeout writes fail,
and Flux reconciles and Helm upgrades time out.

## First checks

```bash
kubectl -n kyverno top pods
kubectl -n kyverno logs deploy/kyverno-admission-controller --since=15m | grep -iE 'slow|timeout|context deadline'
```

```promql
histogram_quantile(0.99, sum by (le, resource_kind) (rate(kyverno_admission_review_duration_seconds_bucket[5m])))
topk(5, sum by (policy_name) (rate(kyverno_validating_policy_execution_duration_seconds_sum[5m])))
```

## Fix

- CPU throttling: raise the admission controller CPU limit in Git.
- One slow policy: a CEL expression or an external lookup (image verification, API call) dominates the
  `topk` query. Narrow its `matchConstraints` or cache the lookup.
- API server slowness shows up here too; check `apiserver_request_duration_seconds`.

## Escalation

Escalate to the platform owner if latency passes 5 seconds.
