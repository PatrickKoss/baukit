# BaukitFalcoDown

## Meaning

The `falco` DaemonSet in namespace `falco` has had at least one scheduled pod that was not ready for
10 minutes.

## Impact

Falco does not watch syscalls on the affected node. Shells, unexpected processes and file writes in
containers on that node raise no events and no alerts.

## First checks

```bash
kubectl -n falco get daemonset falco
kubectl -n falco get pods -l app.kubernetes.io/name=falco -o wide
kubectl -n falco describe pod -l app.kubernetes.io/name=falco
kubectl -n falco logs -l app.kubernetes.io/name=falco -c falco --tail=100
```

PromQL for the gap per DaemonSet:

```promql
kube_daemonset_status_desired_number_scheduled{namespace="falco",daemonset="falco"}
  - kube_daemonset_status_number_ready{namespace="falco",daemonset="falco"}
```

## Fix

- `CrashLoopBackOff` with a rules error in the log (`LOAD_ERR_*`): a rules file in
  `/etc/falco/rules.d` does not compile. Fix `deploy/platform/falco/rules/baukit.rules.yaml` or the
  overlay's `falco-overlay-rules` ConfigMap, run `deploy/platform/falco/validate-rules.sh`, and let Flux
  reconcile.
- `modern_ebpf` fails to load: the node kernel lacks BTF or is older than 5.8. Check
  `uname -r` on the node and `ls /sys/kernel/btf/vmlinux`.
- `OOMKilled`: Falco's thread table grew past the 1Gi limit. Raise the limit in the overlay and record
  the new footprint in `deploy/platform/local-footprint.md`.
- `Pending`: the node lacks room for the `platform-standard` requests. Check
  `kubectl describe node` for allocatable CPU and memory.

## Escalation

If Falco stays down for more than an hour on a production node, treat the node as unmonitored and tell
the platform owner. Do not deploy untrusted workloads to it until Falco is back.
