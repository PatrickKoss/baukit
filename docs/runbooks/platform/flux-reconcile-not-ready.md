# BaukitFluxReconcileNotReady

## Meaning

A Flux Kustomization or HelmRelease has reported `Ready=False` for 15 minutes. The labels
`customresource_kind`, `exported_namespace` and `name` identify it. The signal comes from the
kube-state-metrics `gotk_resource_info` series.

## Impact

The cluster no longer follows Git for that object. A HelmRelease may be stuck mid-upgrade or rolled
back, and every Kustomization that `dependsOn` it waits too.

## First checks

```bash
flux get kustomizations -A --status-selector ready=false
flux get helmreleases -A --status-selector ready=false
kubectl -n <namespace> describe <kind> <name>
kubectl -n flux-system logs deploy/kustomize-controller --since=30m | grep '<name>'
kubectl -n flux-system logs deploy/helm-controller --since=30m | grep '<name>'
```

```promql
gotk_resource_info{ready="False", customresource_kind=~"Kustomization|HelmRelease"}
```

```logql
{namespace="flux-system"} |= "<name>" | json | level="error"
```

## Fix

- Build or schema error in Git: run `flux build kustomization <name> --path <path>` locally, fix the
  manifest and push. Then `flux reconcile kustomization <name> --with-source`.
- HelmRelease upgrade failed: read `status.conditions` and the Helm history
  (`helm -n <namespace> history <release>`). Fix the values in Git; if remediation retries are used up,
  run `flux reconcile helmrelease <name> -n <namespace> --force`.
- Health check timeout: find the unhealthy workload with `kubectl get pods -A | grep -v Running`, fix it,
  and the next reconcile turns Ready.
- Dependency not ready: fix the object named in the `dependsOn` message first.

## Escalation

If the object stays not Ready after a fix is in Git for 30 minutes, or it is a platform base
(`flux-system`, `kyverno`, `cert-manager`, `postgres`), page the platform owner.
