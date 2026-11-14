# BaukitKyvernoAdmissionControllerDown

## Meaning

Deployment `kyverno/kyverno-admission-controller` has had no available replica for 5 minutes.

## Impact

Kyverno's webhooks have no backend. For policies with `failurePolicy: Fail` the API server rejects
matching writes, so deployments and Flux reconciles fail. For `Ignore` policies objects get in without
checks.

## First checks

```bash
kubectl -n kyverno get deploy,pods -o wide
kubectl -n kyverno describe deploy kyverno-admission-controller
kubectl -n kyverno logs deploy/kyverno-admission-controller --since=30m --previous
kubectl get validatingwebhookconfigurations,mutatingwebhookconfigurations | grep kyverno
```

```promql
kube_deployment_status_replicas_available{namespace="kyverno"}
```

```logql
{namespace="kyverno"} | json | level="error"
```

## Fix

- OOMKilled or CrashLoopBackOff: check the resource limits in `deploy/platform/kyverno/helm-release.yaml`
  and the logs; roll back the last Kyverno upgrade in Git if it caused it.
- Pending: the node is out of CPU or memory; free capacity or lower other requests.
- If the cluster is wedged because the webhook blocks the fix itself, delete the Kyverno
  `ValidatingWebhookConfiguration` objects; Kyverno recreates them on start.

## Escalation

Page the platform owner immediately.
