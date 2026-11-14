# BaukitCnpgWalArchivingFailing

## Meaning

`pg_stat_archiver.failed_count` rose in the last 10 minutes and kept rising for 10 more. The barman
cloud plugin cannot push WAL segments to the object store.

## Impact

Point-in-time recovery ends at the last archived segment. Unarchived WAL stays on the volume, which
fills it and eventually stops PostgreSQL.

## First checks

```bash
kubectl cnpg status <cluster> -n <namespace>
kubectl -n <namespace> logs <pod> -c plugin-barman-cloud --since=30m
kubectl -n <namespace> get objectstores.barmancloud.cnpg.io -o yaml
```

```promql
increase(cnpg_pg_stat_archiver_failed_count[1h])
cnpg_pg_stat_archiver_seconds_since_last_archival
```

```logql
{namespace="<namespace>", pod="<pod>"} |= "wal-archive" |= "error"
```

## Fix

- Credentials: the object store Secret was rotated or expired. Update it and the next archive attempt
  succeeds.
- Bucket missing or permission denied: fix the bucket policy.
- Network: the `postgres` NetworkPolicy must allow egress to the object store endpoint.
Archiving catches up on its own once the cause is gone. Take a fresh backup afterwards with
`kubectl cnpg backup <cluster> -n <namespace> --method plugin --plugin-name barman-cloud.cloudnative-pg.io`.

## Escalation

Page the platform owner if archiving fails for an hour or the WAL volume passes 80%.
