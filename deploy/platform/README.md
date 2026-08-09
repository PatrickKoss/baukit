# Baukit platform bases

`deploy/platform` is a menu of independently includable, Flux-native Kustomize
bases for small single-node k3s clusters. It is not a complete distribution.
Cluster overlays compose only the components they need and keep all identity
and secrets in the private infrastructure repository.

## Conventions

Each first-level base owns its namespace manifest and any HelmRepository it
uses. HelmRelease chart versions are exact pins. Values favor one replica and
bounded requests/limits suitable for an initial single Hetzner node; overlays
replace those values from measurements. Bases do not set `priorityClassName`.

The conventional namespaces are:

| Concern | Namespace |
|---|---|
| baseline coordination | `platform-system` |
| certificates | `cert-manager` |
| ingress | `traefik` |
| progressive delivery | `flagger-system` |
| product analytics | `posthog` |
| admission policy | `kyverno` |
| coordinated reboot | `kured` |
| cloud provider drivers | `kube-system` |
| database operator | `cnpg-system` |
| shared database cluster | `postgres` |
| identity | `keycloak` |
| metrics, logs, and traces | `observability` |

`cluster-baseline` defines `platform-critical` and `platform-standard` as
non-default PriorityClasses. Overlays may assign `platform-critical` to the
minimum controllers needed for recovery/ingress and `platform-standard` to
other platform workloads. Application workloads normally use neither.

Every base is secret-free and personal-data-free. Domains, e-mail addresses,
cloud and DNS credentials, object-storage endpoints/credentials, alert
receivers, issuer objects, identity configuration, and cluster inventory are
overlay data. A base README lists its precise overlay contract.

## Consuming bases

A private cluster overlay should reference a tag-pinned baukit GitRepository,
then compose the desired base paths through Flux Kustomizations. Express CRD
ordering with Flux `dependsOn`: for example, reconcile the observability stack
before releases that create ServiceMonitor/PodMonitor objects, and reconcile
cert-manager before applying Certificate or Issuer resources. Optional means
the overlay omits a base; shared bases do not contain enable/disable switches.

For application namespaces, include
`cluster-baseline/app-namespace-policy` as a Kustomize component, set the target
namespace, patch its quota when needed, and supply explicit ingress/egress
allows in the same overlay. The component is not applied globally.

## Validation

Install `kustomize`, `kubeconform`, and `helm`, then run:

```sh
deploy/platform/validate.sh
```

The script discovers every first-level directory containing
`kustomization.yaml`, renders it, validates known Kubernetes resources, and
renders each Flux HelmRelease from its pinned chart. Charts and downloaded core
Kubernetes schemas are cached in `deploy/platform/.helm-cache/`; a warm cache
makes validation offline friendly. `make ci` runs the same validation.

## Adding a base

Create a first-level directory with `kustomization.yaml`, a Namespace, local
HelmRepository and exact-version HelmRelease manifests (or reviewed plain
resources), bounded defaults, and a short README. Do not add identity, secret
values, product-specific behavior, unpinned versions, internal ops exposure, or
an in-base optionality flag. Run the platform validator plus direct `helm lint`
for every included chart.

## Reproducible local-cluster lifecycle

`platform-lifecycle.sh` creates and converges a persistent k3d cluster whose
desired state lives in a separate GitOps repository. It contains no cluster,
person, organization, repository, or credential defaults. The same script is
available through `make platform-{up,down,nuke,recreate,status}`.

### Prerequisites

Install Docker, k3d, Flux, kubectl, SOPS, age, Git, and tar. The lifecycle pins
Flux `v2.9.4` and defaults to the WSL2/cgroup-v1-safe
`rancher/k3s:v1.34.8-k3s1`. The GitOps repository must already contain the
Flux entrypoint at the configured cluster path. Its committed sync manifests
may exist already; bootstrap reuses and reconciles them idempotently.

The SOPS age identity is an out-of-repository file. Keep it mode 0600, back it
up separately, and never put it in the lifecycle configuration file. GitHub
bootstrap additionally reads `GITHUB_TOKEN` from the process environment. For
example, a wrapper can export the result of `gh auth token`; the lifecycle
never persists that token.

### Configuration contract

Pass settings as environment variables or as a trusted shell assignment file
with `--config FILE`. A config file is sourced and must therefore be locally
owned and contain assignments only.

| Variable | Required/default | Meaning |
|---|---|---|
| `PLATFORM_CLUSTER_NAME` | required | Identity-free k3d cluster name. |
| `PLATFORM_GITOPS_URL` | required for `up` | Git repository URL recorded by the cluster sync source. |
| `PLATFORM_GITOPS_BRANCH` | `main` | Branch Flux reconciles. |
| `PLATFORM_GITOPS_PATH` | required for `up` | Repository-relative cluster entrypoint, for example `clusters/local`. |
| `PLATFORM_GITOPS_AUTH_MODE` | `github` | `github` for provider-managed deploy-key bootstrap, or `ssh-key` for a pre-authorized key. |
| `PLATFORM_GITHUB_OWNER` | GitHub mode | GitHub user or organization. |
| `PLATFORM_GITHUB_REPOSITORY` | GitHub mode | Repository name without owner. |
| `PLATFORM_GITHUB_OWNER_TYPE` | `organization` | `organization` or `personal`. |
| `GITHUB_TOKEN` | GitHub mode | Bootstrap-only credential; environment only. |
| `PLATFORM_GITOPS_PRIVATE_KEY_FILE` | SSH-key mode | Existing passwordless key authorized read/write for bootstrap. |
| `PLATFORM_AGE_KEY_FILE` | required for `up` | SOPS age identity installed as `flux-system/sops-age`. |
| `PLATFORM_BAUKIT_SOURCE_MODE` | `github-tag` | `github-tag` or `local-snapshot`. The GitOps overlay owns the matching `GitRepository/baukit` manifest. |
| `PLATFORM_BAUKIT_CHECKOUT` | snapshot mode | Baukit working-tree path copied into the disposable snapshot. |
| `PLATFORM_K3S_IMAGE` | `rancher/k3s:v1.34.8-k3s1` | k3s image used when creating the cluster. |
| `PLATFORM_FLUX_VERSION` | `v2.9.4` | Flux pin; override only as a reviewed platform upgrade. |
| `PLATFORM_SNAPSHOT_PORT` | `9418` | Host port for local smart HTTP. |
| `PLATFORM_STATE_DIR` | XDG state path | Disposable snapshot repository state. |

GitHub mode uses the owner/repository fields to bootstrap and verifies desired
state through the configured branch and path. The URL should be the matching
SSH URL used by the committed `GitRepository`, such as
`ssh://git@github.com/ORGANIZATION/REPOSITORY`. SSH-key mode passes the generic
URL and key directly to `flux bootstrap git`.

In `local-snapshot` mode, the script copies the Baukit working tree without
`.git`, build outputs, or local runtime state, makes a disposable commit, and
serves its bare repository through `git http-backend` in a named Docker
container. Commit metadata is fixed so identical working-tree content retains
the same snapshot revision across repeated `up` calls. The GitOps overlay
should point `GitRepository/baukit` at
`http://host.k3d.internal:9418/baukit.git` on branch `main`. Dumb HTTP is not
supported because Flux's go-git client requires the smart protocol. In
`github-tag` mode no snapshot container is created; the overlay should use the
real Baukit repository and a reviewed `baukit-v*` tag.

### Commands and persistence semantics

```sh
make platform-up PLATFORM_CONFIG=/path/to/local.env
make platform-status PLATFORM_CONFIG=/path/to/local.env
make platform-down PLATFORM_CONFIG=/path/to/local.env
make platform-nuke PLATFORM_CONFIG=/path/to/local.env
make platform-recreate PLATFORM_CONFIG=/path/to/local.env

# Equivalent direct form:
deploy/platform/platform-lifecycle.sh --config /path/to/local.env up
```

- `up` creates or starts k3d, refreshes a configured local snapshot, enforces
  `restart=unless-stopped` on every node/load-balancer/snapshot container,
  persists the snapshot gateway in CoreDNS, installs the age Secret,
  bootstraps pinned Flux and the Git source, kicks reconciliation, and waits
  for every Kustomization and HelmRelease.
- `down` stops the k3d and snapshot containers. Cluster volumes, the
  Kubernetes datastore, and snapshot state remain for the next `up`.
- `nuke` deletes the cluster, k3d-labelled volumes and network, the snapshot
  container/image, and its disposable state directory. The Git repository and
  external age identity are untouched.
- `recreate` runs `nuke` followed by `up`; this is the reproducibility proof.
- `status` shows the cluster, container restart policies, Git sources,
  Kustomizations, HelmReleases, and non-ready pods.

Because `unless-stopped` is enforced after both create and start, Docker or
host restarts bring the cluster and snapshot service back without a lifecycle
command. An intentional `down` remains stopped until `up`, as Docker preserves
the user's explicit stop across daemon restarts.

The lifecycle also owns the `host.k3d.internal.server` entry in k3s's
`kube-system/coredns-custom` ConfigMap. k3d's generated `NodeHosts` gateway
entry can disappear after a container/host restart on some releases; the
custom server entry persists in the Kubernetes datastore so Flux can reacquire
the smart-HTTP snapshot without a repair command.

### Adapting the contract to a remote cluster

Keep the GitOps repository, Flux entrypoint, SOPS secret name, and component
composition. Replace k3d creation with the remote node/bootstrap mechanism,
point kubectl at that cluster, and install Flux plus the age identity there.
Use `PLATFORM_BAUKIT_SOURCE_MODE=github-tag` and change the overlay's Baukit
`GitRepository` from local smart HTTP to the real repository with a pinned
release tag. Remote storage, ingress, DNS, certificates, and cloud identity
remain overlay concerns; the publishable bases do not acquire those values.

## Local integration flavor (development harness)

`overlays/local` composes the platform's own development environment. It uses
k3d with the bundled k3s Traefik disabled, publishes the replacement Traefik
on host ports 80 and 443, installs Flux, and lets the overlay's Flux
Kustomizations reconcile all pinned HelmReleases. The local flavor includes the
baseline, cert-manager and a self-signed `ClusterIssuer`, Traefik, CNPG and one
PostgreSQL cluster, the Keycloak operator and realm import, the observability
stack, and MinIO-backed PostgreSQL backups. The hcloud base is intentionally
omitted because Docker-hosted k3d has neither Hetzner metadata nor cloud
volumes.

This older, self-contained integration harness intentionally remains separate
from the Git-backed lifecycle above. It exercises Baukit's publishable local
overlay with generated throwaway Secrets and no infrastructure repository.
Use it for component/integration development, not for a persistent GitOps
cluster. It expects the pinned `k3d`, `kubectl`, `kustomize`, and `flux` CLIs on
`PATH`, plus Docker:

```sh
deploy/platform/platform-up.sh up
deploy/platform/platform-up.sh status
deploy/platform/platform-up.sh down
```

`up` is a converge operation: rerunning it preserves the cluster and generated
credentials, refreshes a snapshot of the working checkout, reinstalls the
pinned Flux controller manifests idempotently, reapplies runtime Secrets, and
waits for every local Flux Kustomization. `down` deletes the `baukit-local` k3d
cluster and stops the local Git HTTP process. The ignored
`deploy/platform/.local-state/` directory is retained so a later `up` reuses
the same credentials; remove that directory manually only when credential
rotation is intended.

`status` prints the generated Keycloak bootstrap, Grafana, MinIO, and fixture
OIDC credentials on demand. The state directory is mode 0700 and its
`secrets.env` is mode 0600. No Secret manifest or credential value is stored in
the publishable overlay. Kubernetes Secrets for PostgreSQL, Keycloak, Grafana,
MinIO, and the initial product are generated and applied at runtime.

### Integration-harness GitRepository contract

Flux still consumes the normal `GitRepository/baukit` contract. The helper
creates a bare snapshot of the current checkout, including uncommitted and
untracked platform work but excluding `.git`, build outputs, generated fixtures,
and local state. A small read-only smart-HTTP server backed by
`git http-backend` exposes that snapshot on host port 9418; the cluster reaches
it at `http://host.k3d.internal:9418/baukit.git`. This keeps every platform
component on the same Flux-native path used by real overlays and avoids either
publishing a private checkout or bypassing Flux with direct Helm installs.

The local overlay remains offline-buildable: it contains only Flux source and
Kustomization objects, while nested paths and chart fetching are handled at
runtime. `deploy/platform/validate.sh` separately renders every local component
to catch patch and schema errors without needing the Git server or cluster.

### Backups and integration fixture

MinIO runs in the `postgres` namespace with a generated root credential and a
2 GiB local-path PVC. The bootstrap Job creates `baukit-local`, and the CNPG
`ObjectStore` writes under `s3://baukit-local/postgres`. The monthly restore
CronJob is enabled locally so it can be exercised immediately with:

```sh
kubectl create job --from=cronjob/postgres-restore-test \
  postgres-restore-test-manual -n postgres
kubectl wait job/postgres-restore-test-manual -n postgres \
  --for=condition=complete --timeout=35m
```

`overlays/local/fixture-values.yaml` contains only non-secret values for the
generated integration fixture. Its runtime database URL belongs in the
`fixture-runtime` Secret. The values enable the API, migration hook, worker,
ingress, ServiceMonitors, and the local network-policy egress required for
PostgreSQL, Keycloak, and Tempo. See `local-footprint.md` for the measured
complete-stack footprint.

### Local k3s compatibility and troubleshooting

The infrastructure pin is the stable k3s v1.36 channel. k3s v1.36 rejects
cgroup v1 hosts, including the WSL2 Docker environment used for this proof, so
the k3d flavor uses the newest k3s release that still supports that host:
`v1.34.8-k3s1`. Real nodes must use the I0 v1.36 stable pin. Upgrade the local
exception once the development Docker host exposes cgroup v2.

- If `GitRepository/baukit` reports an EOF or connection failure, inspect
  `.local-state/git-http.log`, verify the PID recorded in
  `.local-state/git-http.pid`, and check that host port 9418 is free.
- If the first pull is slow, inspect `flux get helmreleases --all-namespaces`
  and `kubectl get pods --all-namespaces`; `up` waits for reconciliation and is
  safe to rerun.
- If port 80 or 443 is already occupied, stop the conflicting host service
  before creating the cluster. k3d owns both mappings for the cluster's
  lifetime.
- If MinIO bootstrap fails, inspect `job/minio-create-bucket` in `postgres` and
  confirm the runtime `minio-root` Secret exists. The `mc` container uses a
  writable `/tmp/.mc` configuration directory because it runs non-root.
- Use `kubectl get backup,scheduledbackup -n postgres` and the restore Job logs
  for backup problems. A successful restore publishes
  `baukit_restore_test_last_success_timestamp_seconds` through the local
  Pushgateway.
