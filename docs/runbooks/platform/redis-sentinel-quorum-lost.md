# BaukitRedisSentinelQuorumLost

## Meaning

Fewer than 2 of the 3 Sentinel containers in `platform-redis` have been ready for 2 minutes. Sentinel
monitors `mymaster` with quorum 2, so it can no longer agree that the master is down or elect a new
one.

## Impact

Nothing breaks while the master stays up, but the next master loss is an outage instead of a 30 second
failover. Clients resolve the master through `redis-sentinel.platform-redis.svc:26379`, so new
connections fail when no ready Sentinel answers.

## First checks

```bash
kubectl -n platform-redis get pods -o wide
kubectl -n platform-redis describe pod <pod> | grep -A5 sentinel
kubectl -n platform-redis logs <pod> -c sentinel --since=30m | tail -50
```

```promql
kube_pod_container_status_ready{namespace="platform-redis", container="sentinel"}
```

## Fix

- Pods pending or evicted: check node capacity and the PodDisruptionBudget. A drain that takes two pods
  at once leaves one Sentinel.
- Sentinel not ready but running: the readiness probe is `redis-cli -p 26379 ping`, so Sentinel does
  not answer at all. Check its log for config errors, then delete the pod; `redis-ha.sh` writes a fresh
  config from what the other Sentinels report.
- Network policy changes: the Sentinels talk to each other on port 26379. Check the policies in
  `deploy/platform/redis-ha/network-policy.yaml`.

## Escalation

Page the platform owner if the quorum is not back within 30 minutes, or right away when
`BaukitRedisMasterDown` fires at the same time.
