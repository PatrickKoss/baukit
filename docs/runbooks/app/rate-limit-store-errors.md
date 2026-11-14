# BaukitRateLimitStoreErrors

## Meaning

`http_rate_limit_decisions_total{outcome="error"}` has been non-zero for 10 minutes for one
`scope`. The limiter cannot reach or use its Redis store.

## Impact

Depends on `<PREFIX>__RATE_LIMIT__FAIL_MODE`. With the default `open`, requests pass without any
limit, so abuse and quota protection are off. With `closed`, requests in that scope are rejected.

## First checks

Error rate and the scopes affected:

```promql
sum by (scope, outcome) (rate(http_rate_limit_decisions_total{service="<service>", environment="<environment>"}[5m]))
```

Redis pod health (the chart deploys `<release>-redis` when `redis.enabled`):

```sh
kubectl -n <namespace> get pods -l app.kubernetes.io/component=redis
kubectl -n <namespace> logs deploy/<release>-redis --tail=50
```

The store error the limiter logs:

```logql
{service="<service>", environment="<environment>", level="warn"} | json | message="rate-limit store decision failed"
```

## Fix

- Redis is down or OOMKilled: restart it and raise its memory limit if it was killed.
- Wrong URL or password after a secret rotation: correct `<PREFIX>__RATE_LIMIT__REDIS_URL` in the
  product secret and restart the API.
- NetworkPolicy blocks the API from Redis on 6379: restore the egress rule.

## Escalation

Page the product owner. With fail mode `open` this is a security gap, so do not leave it until
morning.
