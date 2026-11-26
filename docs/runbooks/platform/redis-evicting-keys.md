# BaukitRedisEvictingKeys

## Meaning

A Redis pod in `platform-redis` has evicted keys in every 10 minute window for 15 minutes. redis-ha runs
with `--maxmemory 48mb --maxmemory-policy allkeys-lru`, so Redis deletes the least recently used keys
whenever the data set hits the limit.

## Impact

Evicted rate-limit counters start over at zero, so tenants get more requests through than their quota.
Nothing fails outright, which is why this is a warning.

## First checks

```bash
kubectl -n platform-redis exec <pod> -c redis -- redis-cli info memory | grep -E 'used_memory_human|maxmemory_human'
kubectl -n platform-redis exec <pod> -c redis -- redis-cli --bigkeys
```

```promql
sum by (pod) (rate(redis_evicted_keys_total{namespace="platform-redis"}[5m]))
redis_memory_used_bytes{namespace="platform-redis"}
```

Look for one prefix that owns most keys. Products prefix their keys with `<product>-<environment>:`
(the chart's `backing.redis.keyNamespace`), so the prefix names the product.

## Fix

- One product writes far more keys than the others: check that its rate-limit windows expire (every
  key needs a TTL) and that it does not use the shared Redis for other state.
- The total load grew: raise `--maxmemory` in `deploy/platform/redis-ha/redis-ha.sh` together with the
  Redis container memory limit in `statefulset.yaml`, keeping the limit well above maxmemory for the
  replication buffers.

## Escalation

None at night. Raise it with the product owner during working hours.
