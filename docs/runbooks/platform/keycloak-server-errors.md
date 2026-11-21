# BaukitKeycloakServerErrors

## Meaning

More than 5% of the requests Keycloak served in one pod returned 5xx for 10 minutes, with at least
one request every 10 seconds. The metric is Keycloak's `http_server_requests_seconds_count` on the
management port, so the ratio covers every realm on the instance.

## Impact

Sign-ins, token refreshes and JWKS fetches fail for every product realm on this Keycloak. Backends
that cached the JWKS keep verifying tokens until the keys rotate.

## First checks

```bash
kubectl -n keycloak get pods -o wide
kubectl -n keycloak logs statefulset/keycloak --since=15m | grep -E '"level":"(ERROR|WARN)"'
kubectl -n postgres get cluster.postgresql.cnpg.io,database.postgresql.cnpg.io keycloak
```

```promql
sum by (uri, status) (rate(http_server_requests_seconds_count{container="keycloak", outcome="SERVER_ERROR"}[5m]))
```

```logql
{namespace="keycloak", container="keycloak"} | json | level="ERROR"
```

## Fix

- Errors on every URI with JDBC exceptions in the log: the database is down or refuses
  `keycloak_owner`; follow `cnpg-instance-not-ready` and check `keycloak-db-credentials`.
- Errors on one realm's token endpoint after a realm import: revert the realm change in Git; the
  import Job applies the previous file.
- OutOfMemoryError in the log: raise the memory limit in the overlay and keep `JAVA_OPTS_KC_HEAP`
  at 60% of it.

## Escalation

Page the platform owner when every product's sign-in fails. Escalate a single realm to its product
owner.
