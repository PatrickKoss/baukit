# BaukitKuredRebootPending

## Meaning

kured has reported `/var/run/reboot-required` on a node for 48 hours. kured reboots inside its window
and only one node at a time, so it is either blocked or outside its window for two days.

## Impact

Kernel and library security updates are installed but not running.

## First checks

```bash
kubectl -n kured logs -l app.kubernetes.io/name=kured --since=48h | grep -iE 'reboot|blocked|lock'
kubectl -n kured get ds kured -o jsonpath='{.metadata.annotations}'
kubectl get nodes -o wide
```

```promql
kured_reboot_required
ALERTS{alertstate="firing"}
```

## Fix

- Blocked by firing alerts: kured waits while alerts match its `blockingPodSelector` or Prometheus
  filter. Fix those alerts.
- A stale lock on the DaemonSet (`weave.works/kured-node-lock`) from a node that died mid-reboot:
  remove it with `kubectl -n kured annotate ds kured weave.works/kured-node-lock-`.
- Outside the window: check `rebootDays`, `startTime` and `endTime` in the HelmRelease.

## Escalation

Escalate to the platform owner after 72 hours or for a security update rated critical.
