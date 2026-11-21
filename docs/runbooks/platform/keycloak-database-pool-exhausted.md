# BaukitKeycloakDatabasePoolExhausted

## Meaning

Requests in one Keycloak pod have waited for a free connection from its Agroal pool
(`agroal_awaiting_count`) for 5 minutes. The base caps the pool at 10 connections
(`KC_DB_POOL_MAX_SIZE`).

## Impact

Logins, token refreshes and admin calls queue behind the pool and time out. Traefik then reports
504s for the identity host.

## First checks

```bash
kubectl -n keycloak logs statefulset/keycloak --since=15m | grep -i -E 'acquisition|timeout'
kubectl cnpg psql postgres-cluster -n postgres -- -c \
  "select state, count(*) from pg_stat_activity where usename = 'keycloak_owner' group by state"
```

```promql
agroal_active_count{container="keycloak"}
agroal_awaiting_count{container="keycloak"}
```

## Fix

- Many `idle in transaction` sessions: restart the pod (`kubectl -n keycloak rollout restart
  statefulset/keycloak`) and look for a stuck admin or import call in the log.
- Steady load above the pool: raise `KC_DB_POOL_MAX_SIZE` in the overlay, staying under the
  CNPG `max_connections` budget (`cnpg-connections-saturated`).
- Slow queries: check the CNPG instance's CPU and disk first.

## Escalation

Page the platform owner. Sign-in for every product depends on this pool.
