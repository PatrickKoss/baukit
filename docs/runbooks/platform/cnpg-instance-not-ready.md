# BaukitCnpgInstanceNotReady

## Meaning

`cnpg_collector_up` has been 0 for 5 minutes on a CNPG instance. The instance manager runs, but it
cannot connect to PostgreSQL in the same pod.

## Impact

With one instance (the base default) the database is down and every product that uses it fails. With
replicas, CNPG fails over if the primary is affected.

## First checks

```bash
kubectl -n <namespace> get cluster.postgresql.cnpg.io
kubectl cnpg status <cluster> -n <namespace>
kubectl -n <namespace> get pod <pod> -o wide
kubectl -n <namespace> logs <pod> -c postgres --since=30m
```

```promql
cnpg_collector_up
kubelet_volume_stats_used_bytes{namespace="<namespace>"} / kubelet_volume_stats_capacity_bytes{namespace="<namespace>"}
```

```logql
{namespace="<namespace>", pod="<pod>"} | json | level=~"error|fatal|panic"
```

## Fix

- Disk full: see `cnpg-volume-filling`, grow the PVC, and PostgreSQL starts again.
- Crash loop after a config change: revert the `postgresql.parameters` change in Git.
- Recovery in progress after a crash: wait; the log shows `redo` progress.
- Corruption: do not delete the PVC. Restore into a new cluster from the object store (see the
  postgres-cluster README) and switch the product Secret over.

## Escalation

Page the platform owner immediately. Data loss decisions (restore, PVC deletion) need the owner.
