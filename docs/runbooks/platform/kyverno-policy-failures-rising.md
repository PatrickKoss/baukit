# BaukitKyvernoPolicyFailuresRising

## Meaning

A ValidatingPolicy returned more than 10 `fail` results in 30 minutes, for 15 minutes. Something
keeps submitting objects that break it.

## Impact

For a `Deny` policy the writes are rejected, so a deployment or a Flux reconcile is failing in a loop.
For an `Audit` policy non-compliant objects keep entering the cluster.

## First checks

```bash
kubectl get validatingpolicy <policy_name> -o yaml
kubectl get events -A --field-selector reason=PolicyViolation | grep '<policy_name>'
kubectl get policyreports -A -o wide
```

```promql
sum by (resource_namespace, resource_kind, execution_cause) (increase(kyverno_validating_policy_results_total{policy_name="<policy_name>", result="fail"}[30m]))
```

## Fix

- Fix the workload or chart that produces the failing objects; the `resource_namespace` label points at
  the owner.
- If the policy is wrong, fix the policy and its fixtures in `deploy/platform/kyverno-policies/`.
- A real exception gets a `PolicyException` with a reason, which counts against the exception budget.

## Escalation

Escalate to the owner of the namespace in `resource_namespace`.
