# Shared Redis Sentinel HA base

This optional, secret-free base installs three Redis pods, each with a Redis
server and Sentinel sidecar, for expendable shared platform state such as
application-level rate-limit counters. The application contract URL is:

```text
redis+sentinel://redis-sentinel.platform-redis.svc:26379/mymaster
```

Sentinel monitors master name `mymaster` with quorum 2. Sentinel detects an
unavailable master after 5 seconds, uses a 30-second failover timeout, and
synchronizes one replica at a time.

Both containers start through `redis-ha.sh`, which the kustomization ships as
the `redis-ha-scripts` ConfigMap. On start the script asks the Sentinels in the
other pods for the current master:

- One master reported: Redis starts as its replica and Sentinel monitors it.
- No answer after three tries 5 seconds apart: `redis-0` starts as master and
  the other pods replicate from it. This is the cold-start path.
- This pod reported as master: the pod was master and restarted before the
  failover finished. Redis waits up to 125 seconds for the Sentinels to promote
  another pod and starts as master only if they still agree on this pod. A
  Sentinel whose timer stalled for more than 2 seconds sits in TILT mode for
  30 seconds and neither votes nor fails over. When two Sentinels start the
  failover at the same moment, each votes for itself and the election fails.
  Sentinel retries twice the failover timeout (60 seconds) after the first
  try, so the wait covers detection (5 seconds), one TILT period, that retry
  and one more failover timeout for the election and promotion. Starting as
  master earlier would make the replicas resync to the empty data set.
- Different masters reported: Redis keeps retrying, then exits so the kubelet
  restarts it.

Sentinel waits for the local Redis and monitors the master that Redis chose,
so on a cold start no Sentinel monitors a guess. Sentinel rewrites and fsyncs
its config on every vote, and the vote request to the other Sentinel leaves
only after that write, so the config sits on a memory-backed `emptyDir`. On
disk, a slow fsync held the request back until the other Sentinel had voted
for itself. Both containers have a startup
probe that allows 3 minutes for this before the liveness probe takes over.

Every pod announces its stable headless name
(`redis-N.redis-headless.platform-redis.svc.cluster.local`) to Sentinel, with
`announce-hostnames` and `resolve-hostnames` on, so a restarted pod with a new
IP keeps its identity. `test_redis_ha.py` covers the decisions with a stubbed
`redis-cli`, and `deploy/platform/e2e/tests/redis-ha` kills the master on a
live cluster.

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

## Disruption and priority

The pods run as `platform-standard`. A PodDisruptionBudget with
`maxUnavailable: 1` lets a drain evict one pod at a time, so Sentinel keeps its
quorum of 2. On a single node the evicted pod has nowhere to go, so the budget
stops the drain after the first eviction. Use the `redis` base on one-node
clusters.

## Durability and sizing

Persistence is intentionally disabled (`--save ""` and `--appendonly no`).
Redis data, Sentinel configuration, and scratch space are `emptyDir` volumes.
Replication covers an individual pod loss, but a full StatefulSet restart
resets all rate-limit buckets. Do not store sessions, durable queues, or
business data here.

Each Redis container requests 10m CPU / 16 MiB and is limited to 100m CPU /
64 MiB; each Sentinel requests 5m CPU / 8 MiB and is limited to 250m CPU /
32 MiB. At 50m CPU a loaded 4-CPU node throttled Sentinel in 37% of its CFS
periods, and a throttled timer that stalls for more than 2 seconds puts
Sentinel into TILT mode; at 250m it was throttled in under 9%. Redis is capped at 48 MB with `allkeys-lru`. These values target
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
product labels, or product configuration live in this base; the ACL component
below reads a Secret the consumer provides.

## ACL users (opt-in)

Without further composition Redis accepts every command from any client that
the NetworkPolicy lets in. To share one Redis between products, compose
`deploy/platform/components/redis-ha-acl` next to this base and give each
product its own key namespace and ACL user:

```yaml
resources:
  - ../../baukit/deploy/platform/redis-ha
  - redis-ha-acl.enc.yaml   # Secret platform-redis/redis-ha-acl
components:
  - ../../baukit/deploy/platform/components/redis-ha-acl
```

The consumer provides Secret `redis-ha-acl` in `platform-redis` with four keys:

| Key | Content |
|---|---|
| `users.acl` | The Redis ACL file, one `user` line per user, see below. |
| `replication-password` | Cleartext password of user `replication`. |
| `sentinel-password` | Cleartext password of user `sentinel`. |
| `monitor-password` | Cleartext password of user `monitor`. |

The user names are fixed. The passwords in `users.acl` can be cleartext
(`>password`) or SHA-256 hashes (`#<hex>`); the three separate keys must be
cleartext because Redis and Sentinel send them. `users.acl` must contain:

```text
user default on nopass resetkeys resetchannels -@all +ping +info
user replication on >REPLICATION_PASSWORD resetkeys resetchannels -@all +psync +replconf +ping
user sentinel on >SENTINEL_PASSWORD resetkeys resetchannels &__sentinel__:hello -@all +multi +exec +slaveof +replicaof +ping +info +role +publish +subscribe +config|rewrite +client|setname +client|kill +script|kill
user monitor on >MONITOR_PASSWORD resetkeys resetchannels -@all +ping +info +config|get +client|setname +slowlog|get +slowlog|len +latency|latest +latency|histogram
```

and one line per tenant, with its namespace from
`rate_limit.redis_key_namespace`:

```text
user <tenant> on ><password> resetkeys ~<namespace>* resetchannels -@all +eval +role +time +hmget +hset +pexpire +get +set +incrby +pexpireat
```

- `default` keeps only `PING` and `INFO` without a password. The probes run
  `redis-cli ping`, and the Sentinel container reads `INFO replication` from
  its local Redis before it writes its config. Any key access as `default`
  gets `NOPERM`.
- `replication` is what every replica sends as `masteruser`/`masterauth`.
  `PSYNC` and `REPLCONF` are the replication handshake.
- `sentinel` is the user Sentinel authenticates with (`sentinel auth-user`,
  `auth-pass`). The list is the one the Redis Sentinel documentation gives:
  `INFO`, `ROLE` and `PING` to monitor, `SLAVEOF`/`REPLICAOF`, `MULTI`, `EXEC`,
  `CONFIG REWRITE`, `CLIENT KILL` and `SCRIPT KILL` to fail over, and
  `PUBLISH`/`SUBSCRIBE` on `__sentinel__:hello` to find the other Sentinels.
- `monitor` is the exporter's read-only user. It reads no keys.
- A tenant reaches only keys under its namespace. `EVAL` runs the rate-limit
  scripts, and Redis also checks the commands a script calls (`TIME`, `HMGET`,
  `HSET`, `PEXPIRE`, `GET`, `SET`, `INCRBY`, `PEXPIREAT`). `ROLE` lets the
  Sentinel client check that it reached the master. There is no `FLUSHALL`,
  `FLUSHDB`, `CONFIG`, `KEYS`, `SCAN` or `DEBUG`.

An operator user such as `user admin on >PASSWORD ~* &* +@all` is optional.
The e2e overlay has one because its test writes a key and waits for
replication.

Clients authenticate through the Sentinel URL. The credentials apply to the
Redis data nodes; Sentinel itself stays unauthenticated:

```text
redis+sentinel://<tenant>:<password>@redis-sentinel.platform-redis.svc:26379/mymaster
```

Sentinel stays open because Sentinel ACLs would need the same user on every
Sentinel plus `sentinel sentinel-user`/`sentinel-pass` for Sentinel-to-Sentinel
traffic, and a client that can reach Sentinel learns only the master's
address. The NetworkPolicy still limits who reaches port 26379, and nothing
behind Sentinel is readable without a data-node password.

Redis reads `users.acl` at start. After the Secret changes, restart the pods
one at a time (`kubectl -n platform-redis rollout restart statefulset/redis`).
`ACL SAVE` fails because the file is mounted read-only, so do not change users
with `ACL SETUSER` at runtime; they would be lost on restart.

## Metrics

A `redis_exporter` sidecar (v1.97.0, pinned by digest) in every pod reads the local Redis on
127.0.0.1 and serves `redis_*` metrics on port 9121 (`metrics`). The base ships a
PodMonitor for it, and Prometheus in `observability` is the only allowed client
of that port. Order this base after the Kustomization that installs the
Prometheus Operator CRDs.
