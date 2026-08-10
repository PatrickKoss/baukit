# Shared Redis Sentinel HA base

This optional, secret-free base installs three Redis pods, each with a Redis
server and Sentinel sidecar, for expendable shared platform state such as
application-level rate-limit counters. The application contract URL is:

```text
redis+sentinel://redis-sentinel.platform-redis.svc:26379/mymaster
```

Sentinel monitors master name `mymaster` with quorum 2. Pod `redis-0` is the
initial master; the other pods initially replicate from
`redis-0.redis-headless`. Sentinel detects an unavailable master after 5
seconds, uses a 30-second failover timeout, and synchronizes one replica at a
time.

## Choosing a Redis base

Choose `deploy/platform/redis/` for the smallest footprint when a Redis
restart is acceptable and automatic failover is unnecessary. Choose this
`redis-ha` base when continued service after one Redis pod is lost justifies
three Redis and three Sentinel processes. Both bases use the
`platform-redis` namespace and overlap on the `redis` Service and policy names,
so they are mutually exclusive per cluster. Switch a single Flux/Kustomize
composition from one path to the other; do not compose both paths together.

The overlapping `redis` Service remains a normal ClusterIP in both bases,
which avoids an immutable Service change during a switch. In HA mode it can be
used for best-effort direct reads, but it selects masters and replicas and is
not a safe write endpoint. Applications should use the Sentinel URL above so
they discover the current writable master. `redis-headless` exists only for
stable peer DNS, and `redis-sentinel` exposes all Sentinel sidecars on port
26379.

## Durability and sizing

Persistence is intentionally disabled (`--save ""` and `--appendonly no`).
Redis data, Sentinel configuration, and scratch space are `emptyDir` volumes.
Replication covers an individual pod loss, but a full StatefulSet restart
resets all rate-limit buckets. Do not store sessions, durable queues, or
business data here.

Each Redis container requests 10m CPU / 16 MiB and is limited to 100m CPU /
64 MiB; each Sentinel requests 5m CPU / 8 MiB and is limited to 50m CPU /
32 MiB. Redis is capped at 48 MB with `allkeys-lru`. These values target
near-zero traffic and should be patched from measurements when traffic or key
cardinality grows.

## NetworkPolicy client opt-in

The namespace denies ingress and egress by default. Port 6379 and Sentinel
port 26379 accept client traffic only when both the source Namespace and source
Pod carry:

```yaml
baukit.dev/redis-client: "true"
```

Redis pods may reach each other on 6379 and 26379 for replication and Sentinel
gossip, plus cluster DNS for peer-name resolution. No Secret, credentials,
product labels, or product configuration live in this base.
