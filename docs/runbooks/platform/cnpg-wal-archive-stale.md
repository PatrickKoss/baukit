# BaukitCnpgWalArchiveStale

## Meaning

The last successful WAL archive on a primary is older than 15 minutes. CNPG sets `archive_timeout` to
5 minutes, so a healthy primary archives at least that often.

## Impact

A restore would lose everything written since the last archived segment.

## First checks

```bash
kubectl cnpg status <cluster> -n <namespace>
kubectl -n <namespace> exec <pod> -c postgres -- psql -c 'select * from pg_stat_archiver'
kubectl -n <namespace> logs <pod> -c plugin-barman-cloud --since=30m
```

```promql
cnpg_pg_stat_archiver_seconds_since_last_archival / 60
increase(cnpg_pg_stat_archiver_archived_count[30m])
```

## Fix

If `failed_count` also rises, follow `cnpg-wal-archiving-failing`. If nothing fails and nothing
archives, the plugin sidecar is hung: restart the instance pod with
`kubectl cnpg restart <cluster> <pod> -n <namespace>`.

## Escalation

Page the platform owner if no WAL is archived for an hour.
