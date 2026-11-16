# BaukitFalcoK8sAuditDown

## Meaning

The `falco-k8saudit` Deployment in namespace `falco` has had no available replica for 10 minutes. It
receives the API server's audit webhook through the k8saudit plugin on the fixed ClusterIP
`10.43.0.60:9765`.

## Impact

The API server retries each audit batch and then drops it. Secret reads by unexpected service accounts,
exec and attach sessions and ClusterRoleBinding changes raise no Falco events and no alerts. The audit
log on the server node (`/var/log/kubernetes/audit/audit.log`, `{service="kube-audit"}` in Loki) still
records them.

## First checks

```bash
kubectl -n falco get deployment falco-k8saudit
kubectl -n falco get pods -l app.kubernetes.io/instance=falco-k8saudit -o wide
kubectl -n falco describe pod -l app.kubernetes.io/instance=falco-k8saudit
kubectl -n falco logs -l app.kubernetes.io/instance=falco-k8saudit -c falcoctl-artifact-install --tail=50
kubectl -n falco logs -l app.kubernetes.io/instance=falco-k8saudit -c falco --tail=100
kubectl -n falco get service falco-k8saudit-webhook
```

## Fix

- `Init:Error` or `Init:CrashLoopBackOff` in `falcoctl-artifact-install`: the init container cannot pull
  the k8saudit plugin from ghcr.io. Check DNS and the `allow-internet-https` NetworkPolicy in `falco`,
  and that the digest in `deploy/platform/falco/k8saudit-helm-release.yaml` still exists upstream. A
  signature error means the artifact does not match its cosign signature; do not bypass it.
- `CrashLoopBackOff` with `LOAD_ERR_*` in the falco log: a rule in
  `deploy/platform/falco/k8saudit-rules/` does not compile against the plugin. Run
  `deploy/platform/falco/validate-rules.sh` and let Flux reconcile.
- The Service has no `10.43.0.60` ClusterIP or another Service holds it: the cluster's service CIDR is
  not the k3s default `10.43.0.0/16`. Change `clusterIP` in the release and the server in
  `deploy/platform/k3s/audit-webhook.yaml` (and the node's copy of it) together.
- `OOMKilled`: a burst of audit events outgrew the 256Mi limit. Raise it in the overlay and record the
  footprint in `deploy/platform/local-footprint.md`.

## Escalation

If audit detection stays down for more than an hour on a production cluster, tell the platform owner
and review the audit log in Loki for the window by hand.
