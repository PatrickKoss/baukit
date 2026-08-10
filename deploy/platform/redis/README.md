# Shared Redis base

This optional, secret-free base installs one shared Redis instance for
expendable platform state such as application-level rate-limit counters. It is
inert unless a cluster overlay explicitly composes `deploy/platform/redis/`.

Products choose between this shared component and the optional per-product
Redis in the `baukit-app` chart. Choose the chart Redis when a product should
own and lifecycle-isolate its rate-limit state. Choose this base when several
products deliberately share one platform Redis and accept the shared failure,
capacity, and trust boundary. Do not enable both for the same product.

## Service and durability contract

Set the consuming product's `<PREFIX>__RATE_LIMIT__REDIS_URL` to exactly:

```text
redis://redis.platform-redis.svc:6379
```

The equivalent fully qualified cluster DNS name is
`redis.platform-redis.svc.cluster.local`. The Service is cluster-internal and
Redis has no password; the NetworkPolicy opt-in described below is the access
boundary.

Persistence is intentionally disabled (`--save ""` and `--appendonly no`). The
only `/data` volume is an `emptyDir`, so a pod reschedule, restart, or base
removal loses all keys. That is acceptable for rate-limit counters and other
reconstructible state; do not store sessions, durable queues, or business data
here.

The default 10m CPU / 16 MiB request and 100m CPU / 64 MiB limit are the
near-zero-traffic envelope. A local idle sample of the pinned image used about
8.3 MiB and less than 10m CPU; Redis is capped at 48 MB with `allkeys-lru` so
expendable keys are evicted before the container limit. Re-measure and patch
these values when observed traffic or key cardinality requires it.

## NetworkPolicy client opt-in

The Redis namespace denies all ingress and egress by default. Redis itself
needs no egress. Port 6379 accepts traffic only when both the source Namespace
and the source Pod have this label:

```yaml
baukit.dev/redis-client: "true"
```

Requiring the label at both levels follows the sibling PostHog base's combined
`namespaceSelector` plus `podSelector` allow-policy convention. Add both labels
through the product's desired-state manifests; labeling only a namespace or
only a pod does not grant access.

## Overlay composition

Compose this base with its own Flux `Kustomization`, ordered after the cluster
baseline. For example:

```yaml
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: shared-redis
  namespace: flux-system
spec:
  dependsOn:
    - name: cluster-baseline
  interval: 10m
  retryInterval: 30s
  timeout: 10m
  path: ./deploy/platform/redis
  prune: true
  wait: true
  sourceRef:
    kind: GitRepository
    name: baukit
```

The referenced Baukit `GitRepository` remains the overlay's tag-pinned source.
No Secret, credentials, product labels, or product configuration live in this
base.
