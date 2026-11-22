# Keycloak identity base

This optional base runs one Keycloak 26.8.2 (`quay.io/keycloak/keycloak`, pinned by digest) in the
`keycloak` namespace, with its database on the shared CNPG cluster. It is inert unless a cluster
overlay composes `deploy/platform/keycloak/`. Production products sign in with Clerk or WorkOS
(BK21); this base exists for shared non-production clusters such as a home lab, where each product
gets one realm.

It is a plain StatefulSet, not the Keycloak Operator. The operator needs about 18,000 lines of
vendored CRDs and a second controller pod (261 MiB on the old local cluster) for one instance, and a
StatefulSet passes the current Kyverno policy set without an exception.

## What the base ships

| Object | Purpose |
|---|---|
| `Namespace/keycloak` | PSA `restricted`, `baukit.dev/postgres-client` and `baukit.dev/ingress-backend` |
| `DatabaseRole/keycloak-owner`, `Database/keycloak` in `postgres` | the CNPG role and database, both `retain` |
| `StatefulSet/keycloak` | one pod, `start --optimized`, read-only root filesystem |
| `Service/keycloak` (8080), `Service/keycloak-management` (9000) | HTTP and the health/metrics port |
| `ServiceMonitor/keycloak` | scrapes `/metrics` on the management port |
| `Job/keycloak-realm-import` | keycloak-config-cli 6.5.1 (pinned by digest) applies the realm files |
| `ConfigMap/keycloak-realms` | generated, empty in the base; the overlay merges realm files into it |
| NetworkPolicies | default deny, DNS, the allows below |

Three init containers prepare the read-only pod. `wait-for-database` runs `select 1` against
database `keycloak` as the Keycloak user, every 2 s for up to 5 minutes, because the `Database` and
`DatabaseRole` apply together with the StatefulSet and CNPG creates them a few seconds later. It
uses the CNPG PostgreSQL image the platform already pins. `seed` copies the image's `lib/quarkus` into an
`emptyDir`, and `build` runs `kc.sh build` with the build-time options (PostgreSQL, health, metrics,
user event metrics, local cache) into it. The server then starts with `--optimized` and writes only
to its three `emptyDir` volumes. `KC_CACHE=local` keeps the single instance off JGroups, so the
namespace needs no clustering ports. Scaling past one replica needs a distributed cache and is out
of scope.

The alerts live in the platform observability pack (`deploy/observability-platform/alerts/keycloak.rules.yml`,
component `keycloak`). They select `container="keycloak"`, so they stay quiet on clusters without
this base. Scrape failures are covered by `BaukitPlatformTargetDown`.

## Overlay requirements

An overlay must:

- reconcile `cnpg` and the `postgres-cluster` instance first, through Flux `dependsOn`;
- create `keycloak-db-credentials` (type `kubernetes.io/basic-auth`, `username: keycloak_owner`
  and a generated `password`) in both `postgres` (for the `DatabaseRole`) and `keycloak` (for the
  pod);
- create `keycloak-bootstrap-admin` in `keycloak` with `username` and `password`. Keycloak creates
  the temporary master admin from it on the first start, and the import Job logs in with it;
- replace `KC_HOSTNAME` (`https://keycloak.example.invalid` in the base) with the public issuer
  origin, for example `https://id.lab.example.com`. Tokens carry this issuer;
- route the public host to `Service/keycloak` port 8080 through Traefik, and keep `/admin` behind an
  allowlist. The base ships no route;
- optionally create `keycloak-realm-secrets` in `keycloak`. The import Job loads it as environment
  variables, so a realm file can reference a value as `$(env:NAME)`.

Products that call Keycloak in cluster (JWKS, token exchange) label both their namespace and their
pods `baukit.dev/keycloak-client: "true"`. Everything else reaches it through Traefik.

## Realms

Each product's realm is one JSON file in `ConfigMap/keycloak-realms`. The overlay merges its files
into the base's generated ConfigMap:

```yaml
configMapGenerator:
  - name: keycloak-realms
    namespace: keycloak
    behavior: merge
    files:
      - product.json=realms/product.json
```

The generator's hash suffix lands in the Job's pod template, so any realm change gives the Job a new
spec, and Flux replaces it (`kustomize.toolkit.fluxcd.io/force: enabled`). keycloak-config-cli is
idempotent: it creates a missing realm and updates an existing one to match the file. It does not
delete realms whose file was removed. With no file it logs `keycloak-realms holds no realm file`
and exits 0.

Validate a realm for the shared cluster before it lands here:

```sh
python3 scripts/keycloak_policy.py --realm keycloak/realm.lab.json \
  --policy keycloak/realm-policy.lab.json --environment-class lab
```

The `lab` class rejects default, known and literal passwords (use `$(env:NAME)`), loopback and
plain-HTTP redirect URIs, public clients without PKCE S256, and an MCP audience mapper that does not
name the policy's `mcpResourceUrl`.

## NetworkPolicy

| Policy | Allows |
|---|---|
| `allow-keycloak-ingress` | 8080 from Traefik, the import Job, and pods labelled `baukit.dev/keycloak-client` in namespaces with the same label |
| `allow-prometheus-scrape` | 9000 from Prometheus in `observability` |
| `allow-keycloak-postgres-egress` | Keycloak to CNPG instances in `postgres` on 5432 |
| `allow-realm-import-egress` | the import Job to Keycloak on 8080 |

The postgres side admits Keycloak through `baukit.dev/postgres-client` on the namespace and the pod.

## Resources

The pod requests 100m CPU and 640 MiB and may use 1 CPU and 1 GiB; the heap is 60% of the limit
(`JAVA_OPTS_KC_HEAP`). See `../local-footprint.md` for the platform-e2e measurement. The startup
probe allows 5 minutes, because the first start migrates the empty database.

## Overlay composition

```yaml
apiVersion: kustomize.toolkit.fluxcd.io/v1
kind: Kustomization
metadata:
  name: keycloak
  namespace: flux-system
spec:
  dependsOn:
    - name: postgres
    - name: observability
    - name: kyverno-policies
  interval: 10m
  retryInterval: 30s
  timeout: 15m
  path: ./clusters/lab/keycloak
  prune: true
  wait: true
  sourceRef:
    kind: GitRepository
    name: platform
```

`clusters/lab/keycloak` is the overlay directory that references this base, merges the realm files
and patches `KC_HOSTNAME`. `e2e/overlay/components/keycloak` is a working example.
