# BaukitKyvernoEnforcedPolicyViolations

## Meaning

Policy Reporter lists failing results for a policy whose `validationActions` include `Deny`. The
objects predate the policy, or a background scan found drift.

## Impact

The objects keep running but their next update is rejected, which breaks the next deploy. The cluster
also does not meet the policy it claims to enforce.

## First checks

```bash
kubectl get policyreports -A -o json | jq -r '.items[] | .results[]? | select(.policy=="<policy>" and .result=="fail") | .resources[]? | "\(.namespace)/\(.kind)/\(.name)"'
kubectl get validatingpolicy <policy> -o yaml
```

```promql
sum by (exported_namespace, kind, name) (policy_report_result{policy="<policy>", status="fail"})
```

## Fix

- Update the listed objects in Git so they comply; Flux applies them and the report clears.
- An object that must stay as it is gets a `PolicyException` with a reason; the exception count may
  only go down later.

## Escalation

Escalate to the owners of the listed namespaces. Escalate to the platform owner if the violations are
in platform namespaces.
