# BaukitRedisMasterDown

## Meaning

The redis-ha exporters in `platform-redis` still answer, but none of the Redis pods has reported
`role:master` for 2 minutes. Sentinel normally promotes a replica within about 30 seconds of losing the
master, so the failover is stuck or the pods disagree about who the master is.

## Impact

Writes fail. Products that keep rate-limit counters in the shared Redis take the fail-open path, so
quotas are not enforced. Reads from replicas still work for clients that use them.

## First checks

```bash
kubectl -n platform-redis get pods -o wide
for pod in redis-0 redis-1 redis-2; do
  kubectl -n platform-redis exec "$pod" -c redis -- redis-cli info replication | grep -E '^(role|master_host|master_link_status)'
  kubectl -n platform-redis exec "$pod" -c sentinel -- redis-cli -p 26379 sentinel get-master-addr-by-name mymaster
done
```

```promql
redis_instance_info{namespace="platform-redis"}
```

Check whether `BaukitRedisSentinelQuorumLost` fires too. Without two Sentinels there is no failover.

## Fix

- Sentinels agree on a master that is not running: wait for Sentinel to fail over. If it does not
  within a minute, check the Sentinel logs for `-failover-abort` and TILT mode, which pause the vote.
- Sentinels report different masters: delete the pod whose Sentinel disagrees with the other two.
  `redis-ha.sh` asks the other Sentinels on start and rejoins as their replica.
- Every pod runs as a replica of a dead address: restart the pods one at a time, starting with
  `redis-0`. The cold-start path in `deploy/platform/redis-ha/README.md` makes `redis-0` the master
  when nobody answers. The data is expendable rate-limit state.

## Escalation

Page the platform owner if no master is back within 15 minutes or if every pod restart ends in the
same state.
