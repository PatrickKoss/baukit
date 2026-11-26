# BaukitRedisReplicationLinkDown

## Meaning

A Redis replica in `platform-redis` has reported `master_link_status:down` for 5 minutes. It cannot
reach its master or keeps failing the full resync.

## Impact

The replica serves old data, and Sentinel may promote it with that old data if the master dies. Rate
limit counters are expendable, so the real risk is the lost failover target: with one healthy replica
left, a second failure means no master.

## First checks

```bash
kubectl -n platform-redis exec <pod> -c redis -- redis-cli info replication
kubectl -n platform-redis logs <pod> -c redis --since=30m | grep -iE 'master|sync|replica'
```

```promql
redis_master_link_up{namespace="platform-redis"}
```

The `master_host` label shows the address the replica tries. Compare it with the master the Sentinels
report (`sentinel get-master-addr-by-name mymaster`).

## Fix

- The replica points at an old master address after a failover: delete the replica pod. On start it
  asks the Sentinels for the current master.
- The full resync fails with an out-of-memory or `client-output-buffer-limit` message: the data set
  grew past what the master can buffer. Check `BaukitRedisMemoryNearLimit` and the pod memory limit.
- The master is unreachable from this pod only: check the node and the network policy.

## Escalation

Page the platform owner if two replicas lose their link at once or the link stays down after the pod
restarts.
