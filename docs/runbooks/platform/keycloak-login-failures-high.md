# BaukitKeycloakLoginFailuresHigh

## Meaning

Keycloak recorded more than 10 failed logins a minute in one realm for 10 minutes. The count comes
from `keycloak_user_events_total{event="login"}` with a non-empty `error`, which the base turns on
with `KC_EVENT_METRICS_USER_ENABLED`.

## Impact

A password-guessing run locks accounts once brute-force protection trips, so real users of that
realm cannot sign in. A client sending stale credentials looks the same and breaks that client.

## First checks

```bash
kubectl -n keycloak logs statefulset/keycloak --since=15m | grep -i 'type="LOGIN_ERROR"'
```

```promql
sum by (realm, error) (rate(keycloak_user_events_total{event="login", error!=""}[10m])) * 60
```

```logql
{namespace="keycloak", container="keycloak"} |= "LOGIN_ERROR" | json
```

## Fix

- One `ipAddress` or one client in the log: block the source at the edge, or fix and redeploy
  the client's secret.
- Spread over many sources: confirm `bruteForceProtected` is true for the realm
  (`keycloak_policy.py` requires it) and tighten the edge rate limit for `/realms/<realm>/`.
- `error="invalid_user_credentials"` after a password reset campaign: expected, wait it out.

## Escalation

Escalate to the product owner of the realm. Page the platform owner when several realms show it at
once.
