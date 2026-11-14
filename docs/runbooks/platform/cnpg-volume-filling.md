# BaukitCnpgVolumeFilling

## Meaning

A CNPG data or WAL volume is more than 80% full.

## Impact

PostgreSQL stops when the data volume fills and cannot write WAL when the WAL volume fills. Both mean
an outage.

## First checks

```bash
kubectl -n <namespace> get pvc <persistentvolumeclaim>
kubectl -n <namespace> exec <pod> -c postgres -- du -sh /var/lib/postgresql/data/pgdata/pg_wal
kubectl -n <namespace> exec <pod> -c postgres -- psql -c \
  "select datname, pg_size_pretty(pg_database_size(datname)) from pg_database"
```

```promql
kubelet_volume_stats_used_bytes{persistentvolumeclaim="<persistentvolumeclaim>"} / kubelet_volume_stats_capacity_bytes{persistentvolumeclaim="<persistentvolumeclaim>"}
predict_linear(kubelet_volume_stats_used_bytes{persistentvolumeclaim="<persistentvolumeclaim>"}[6h], 86400)
```

## Fix

- WAL grows because archiving fails: fix that first (`cnpg-wal-archiving-failing`).
- A replication slot holds WAL: drop the inactive slot.
- Real data growth: raise `storage.size` (or `walStorage.size`) in the Cluster spec in Git. CNPG resizes
  the PVCs when the storage class allows expansion.

## Escalation

Page the platform owner above 90% or when `predict_linear` shows a full volume within a day.
