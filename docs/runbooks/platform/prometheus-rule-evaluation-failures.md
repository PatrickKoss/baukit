# BaukitPrometheusRuleEvaluationFailures

## Meaning

Prometheus failed to evaluate rules in a group for 15 minutes. The `rule_group` label names the file
and group.

## Impact

Alerts and recording rules in that group do not run. An outage they should catch stays silent.

## First checks

```bash
kubectl -n observability logs prometheus-<name>-0 -c prometheus --since=30m | grep -i 'rule'
kubectl get prometheusrules -A
```

```promql
sum by (rule_group) (increase(prometheus_rule_evaluation_failures_total[30m]))
```

## Fix

- `many-to-many matching not allowed`: the query joins on labels that are not unique. Fix it in the rule
  file and add a promtool test with the duplicate series.
- `query timed out`: the rule is too expensive; add a recording rule.
- After fixing, run `sh scripts/ci-gate.sh --step observability-tests`.

## Escalation

Escalate to the owner of the rule file.
