# BaukitRedisMemoryNearLimit

## Meaning

A Redis pod in `platform-redis` has used more than 90% of its `maxmemory` (48 MB in redis-ha) for
15 minutes. Past the limit Redis evicts keys (`allkeys-lru`).

## Impact

None yet. Once the limit is reached `BaukitRedisEvictingKeys` follows and rate-limit counters start
over. A replica close to the limit also needs memory for a full resync after a failover.

## First checks

```bash
kubectl -n platform-redis exec <pod> -c redis -- redis-cli info memory
kubectl -n platform-redis exec <pod> -c redis -- redis-cli info keyspace
```

```promql
redis_memory_used_bytes{namespace="platform-redis"} / redis_memory_max_bytes{namespace="platform-redis"}
```

## Fix

- Growth from one product: find its key prefix with `redis-cli --bigkeys` and check that its keys
  carry a TTL.
- Steady growth across products: raise `--maxmemory` in `deploy/platform/redis-ha/redis-ha.sh` and the
  Redis container memory limit in `statefulset.yaml` together, then roll the StatefulSet.

## Escalation

None. Plan the capacity change during working hours.
