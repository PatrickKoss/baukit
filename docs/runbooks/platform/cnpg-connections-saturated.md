# BaukitCnpgConnectionsSaturated

## Meaning

A PostgreSQL instance has used more than 85% of `max_connections` for 10 minutes.

## Impact

At 100% new connections fail with `FATAL: sorry, too many clients already`. Requests fail and
migrations cannot connect.

## First checks

```bash
kubectl -n <namespace> exec <pod> -c postgres -- psql -c \
  "select usename, application_name, state, count(*) from pg_stat_activity group by 1,2,3 order by 4 desc"
```

```promql
sum by (usename, application_name, state) (cnpg_backends_total{pod="<pod>"})
cnpg_pg_settings_setting{name="max_connections", pod="<pod>"}
cnpg_backends_max_tx_duration_seconds{pod="<pod>"}
```

## Fix

- Many `idle in transaction` sessions: a product leaks transactions. Find it by `application_name`
  and fix the code; as a stopgap set `idle_in_transaction_session_timeout`.
- A pool sized too large: lower the product's pool size so the sum across replicas stays below
  `max_connections`.
- Real load: raise `max_connections` in the Cluster `postgresql.parameters` (needs a restart) or add a
  PgBouncer `Pooler`.

## Escalation

Escalate to the product owner whose `application_name` holds most connections; to the platform owner
if it is a platform component.
