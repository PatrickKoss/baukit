# BaukitCnpgBackupStale

## Meaning

The newest base backup in the object store is older than 26 hours. The daily ScheduledBackup missed
at least one run.

## Impact

Recovery needs a base backup plus every WAL segment since. An old base backup makes restores slow and
means WAL retention must cover a longer window.

## First checks

```bash
kubectl -n <namespace> get scheduledbackups,backups.postgresql.cnpg.io
kubectl -n <namespace> describe backup <latest-backup>
kubectl -n <namespace> logs <primary-pod> -c plugin-barman-cloud --since=26h | grep -i backup
```

```promql
(time() - barman_cloud_cloudnative_pg_io_last_available_backup_timestamp) / 3600
barman_cloud_cloudnative_pg_io_last_failed_backup_timestamp
```

## Fix

- The last Backup is `failed`: read its `status.error`; credentials and bucket access are the usual
  cause. Fix and run a backup by hand (see `cnpg-wal-archiving-failing`).
- No Backup objects were created: the ScheduledBackup is suspended or its schedule is wrong.
- Backups succeed but the metric is old: the plugin on the primary is not reporting; restart the pod.

## Escalation

Page the platform owner if no backup succeeds within 48 hours.
