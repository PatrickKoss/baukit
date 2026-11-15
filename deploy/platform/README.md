# Baukit platform bases

`deploy/platform` is a menu of independently includable, Flux-native Kustomize
bases for small single-node k3s clusters. It is not a complete distribution.
Cluster overlays compose only the components they need and keep all identity
and secrets in the private infrastructure repository.

## Conventions

Each first-level base owns its namespace manifest and the chart sources it
uses (see [Chart sources](#chart-sources)). HelmRelease chart versions are exact pins. Values favor one replica and
bounded requests/limits suitable for an initial single Hetzner node; overlays
replace those values from measurements.

Every pod a base renders sets `priorityClassName`. `platform-critical` goes to
the controllers needed to recover the cluster and serve traffic: Traefik, the
CNPG operator and the PostgreSQL instance, Kyverno admission, cert-manager and
the hcloud CSI driver. Every other platform pod runs as `platform-standard`.
The hcloud CCM keeps its chart's `system-cluster-critical`. The Flux controllers run as `system-cluster-critical` too. `flux install` sets it
on source-, kustomize- and helm-controller, and `components/flux-lockdown` adds it to
notification-controller, which starts before cluster-baseline creates the platform classes. A chart without a
values key gets the class from a `postRenderers` patch, which `validate.sh`
applies to its render too. Every workload that can run more than one replica
has a PodDisruptionBudget that allows at least one eviction. The convention
rules `priority-class` and `pdb-multi-replica` enforce both.

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
| cloud provider drivers | `kube-system` (owned by k3s, not managed by a base) |
| database operator | `cnpg-system` |
| shared database cluster | `postgres` |
| shared Redis (direct or Sentinel HA) | `platform-redis` |
| metrics, logs, and traces | `observability` |
| node-level agents (node-exporter, Alloy) | `observability-agents` |

`cluster-baseline` defines `platform-critical` and `platform-standard` as
non-default PriorityClasses. Application workloads normally use neither.

Every Namespace a base manages carries `app.kubernetes.io/part-of:
baukit-platform`. Prometheus reads PrometheusRules, ServiceMonitors and
PodMonitors only from namespaces with that label, and Grafana loads dashboard
ConfigMaps labelled `grafana_dashboard: "1"` from every namespace, filed into
the folder named by their `grafana_folder` annotation. Namespaces that hold
volumes or a database (`postgres`, `observability`, `platform-redis`,
`posthog`), the CNPG `Cluster` and Flux-applied PVCs carry
`kustomize.toolkit.fluxcd.io/prune: disabled`, so removing a base from a Flux
Kustomization never deletes data.

Every Flux object follows the rules in `lint/ci-policies/`, which
`validate.sh` enforces on every rendered base and overlay:

- A HelmRelease pins an exact chart version and sets `spec.timeout`,
  `install.remediation.retries`, `upgrade.remediation.retries`,
  `upgrade.cleanupOnFail: true` and `driftDetection.mode` (`warn` today).
- A chart with a Helm `crds/` directory gets `install.crds` and `upgrade.crds:
  CreateReplace`. A chart that renders Helm test hooks gets `test.enable: true`.
  `lint/ci-policies/chart-contents.toml` records both facts per pinned chart.
- A HelmRepository or OCIRepository polls no more often than every hour.
- A Flux Kustomization sets `prune: true` and `wait: true` and has no
  `healthChecks` list, which Flux ignores once `wait` is on.

Timeouts are about three times the install time measured on the local k3d
cluster, rounded up to whole minutes, and at least 5m.

Every base is secret-free and personal-data-free. Domains, e-mail addresses,
cloud and DNS credentials, object-storage endpoints/credentials, alert
receivers, issuer objects, identity configuration, and cluster inventory are
overlay data. A base README lists its precise overlay contract.

## Chart sources

A chart comes from an `OCIRepository` and the HelmRelease points at it with
`spec.chartRef` whenever upstream publishes the pinned version as an OCI
artifact. The OCIRepository lives in the HelmRelease namespace, is named after
the chart, and pins `ref.tag` (the chart version) plus `ref.digest`, which
Renovate updates together. When upstream signs the chart with cosign keyless,
the OCIRepository sets `verify.provider: cosign` with the exact
`matchOIDCIdentity` below, and source-controller refuses an artifact signed by
anyone else. `lint/check_chart_sources.py` fails the validation when a
HelmRelease disagrees with this table.

| Chart | Source | Signed | Flux verify | Identity | Reason |
|---|---|---|---|---|---|
| `cert-manager` | OCI `oci://quay.io/jetstack/charts/cert-manager` | key, RSA over SHA-512 | none |  | Signed with the key in `cert-manager-pubkey-2021-09-20.pem`. openssl verifies the v1.21.3 signature only as RSA PKCS#1 v1.5 over SHA-512, and Flux 2.9.4 reports `no matching signatures` for it with that key, so no verify block. |
| `cloudnative-pg` | OCI `oci://ghcr.io/cloudnative-pg/charts/cloudnative-pg` | keyless | cosign | `^https://token\.actions\.githubusercontent\.com$` `^https://github\.com/cloudnative-pg/charts/\.github/workflows/release-publish\.yml@refs/heads/main$` |  |
| `falco` | OCI `oci://ghcr.io/falcosecurity/charts/falco` | keyless | cosign | `^https://token\.actions\.githubusercontent\.com$` `^https://github\.com/falcosecurity/charts/\.github/workflows/release\.yml@refs/heads/master$` |  |
| `flagger` | OCI `oci://ghcr.io/fluxcd/charts/flagger` | keyless | cosign | `^https://token\.actions\.githubusercontent\.com$` `^https://github\.com/fluxcd/flagger/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$` |  |
| `hcloud-cloud-controller-manager` | HelmRepository `https://charts.hetzner.cloud` | no | none |  | Hetzner publishes no OCI chart. 1.39.3 is not on `ghcr.io/hetznercloud` or Docker Hub, and the release workflows run no `helm push`. |
| `hcloud-csi` | HelmRepository `https://charts.hetzner.cloud` | no | none |  | Same as the cloud controller: 2.24.2 exists only in the HTTPS index. |
| `kured` | OCI `oci://ghcr.io/kubereboot/charts/kured` | no | none |  | No cosign signature or Sigstore bundle exists for the 6.1.0 digest. |
| `kyverno` | OCI `oci://ghcr.io/kyverno/charts/kyverno` | no | none |  | Kyverno signs its images, not the 3.10.0 chart: no signature tag or bundle for the digest. |
| `kyverno-policies` | OCI `oci://ghcr.io/kyverno/charts/kyverno-policies` | no | none |  | Same as the kyverno chart. |
| `policy-reporter` | OCI `oci://ghcr.io/kyverno/charts/policy-reporter` | keyless | cosign | `^https://token\.actions\.githubusercontent\.com$` `^https://github\.com/kyverno/policy-reporter/\.github/workflows/release-chart\.yaml@refs/tags/policy-reporter-chart-v[0-9]+\.[0-9]+\.[0-9]+$` |  |
| `kube-prometheus-stack` | OCI `oci://ghcr.io/prometheus-community/charts/kube-prometheus-stack` | no | none |  | No signature or bundle for the 88.6.5 digest. |
| `loki` | OCI `oci://ghcr.io/grafana-community/helm-charts/loki` | no | none |  | grafana-community pushes charts unsigned. |
| `tempo` | OCI `oci://ghcr.io/grafana-community/helm-charts/tempo` | no | none |  | grafana-community pushes charts unsigned. |
| `alloy` | HelmRepository `https://grafana.github.io/helm-charts` | no | none |  | `ghcr.io/grafana/helm-charts/alloy` holds only 1.16.0, not the pinned 1.13.0. Move it to OCI with the next version bump. |
| `traefik` | OCI `oci://ghcr.io/traefik/helm/traefik` | no | none |  | No signature or bundle for the 41.6.1 digest. |
| `posthog` | HelmRepository `https://posthog.github.io/charts-clickhouse` | no | none |  | PostHog publishes the sunset chart only in the HTTPS index. |

Every keyless identity uses the issuer and subject regexes shown. The subjects
were read from the signing certificate of the pinned digest, not from docs.
A cold `validate.sh` run pulls the OCI charts from `ghcr.io` and `quay.io`
and the remaining charts from their HTTPS indexes. A warm
`deploy/platform/.helm-cache` needs no network for charts.

## Pod Security Admission

Every Namespace manifest carries the four Pod Security Admission labels.
`enforce` is the level below, `enforce-version` is pinned to the k3s minor in
`platform-lifecycle.sh` (`v1.34`), and `audit` and `warn` are `restricted`
everywhere, so a gap shows up as a warning even where `enforce` is looser.

| Level | Namespaces |
|---|---|
| `restricted` | `cert-manager`, `cnpg-system`, `flagger-system`, `flux-system`, `kyverno`, `observability`, `platform-redis`, `platform-system`, `postgres`, `traefik`, tenants |
| `baseline` | `posthog` |
| `privileged` | `kube-system`, `kured`, `observability-agents`, `falco` |

Node-exporter and Alloy need hostPID, hostNetwork or hostPath, so they run in
`observability-agents` and `observability` stays `restricted`. The Kyverno
policy `require-psa-labels` rejects a Namespace without an `enforce` label and
allows `privileged` only for the four namespaces above.
`lint/test_psa_labels.py` checks the manifests offline. The lifecycle labels
`flux-system` before `flux bootstrap`.

## Consuming bases

A private cluster overlay should reference a tag-pinned baukit GitRepository,
then compose the desired base paths through Flux Kustomizations. Express CRD
ordering with Flux `dependsOn`: for example, reconcile the observability stack
before releases that create ServiceMonitor/PodMonitor objects, and reconcile
cert-manager before applying Certificate or Issuer resources. Optional means
the overlay omits a base; shared bases do not contain enable/disable switches.

An overlay that composes `kyverno-policies` orders it right after the
foundation, `kyverno` and then `kyverno-policies`, and makes every other
Kustomization that creates workloads depend on it. Otherwise Kyverno never
admits those workloads on a fresh cluster. `kyverno-monitoring` comes after
both the policies and the observability stack. `lint/check_admission_order.py`
checks this offline.

For application namespaces, include
`cluster-baseline/app-namespace-policy` as a Kustomize component, set the target
namespace, patch its quota when needed, and supply explicit ingress/egress
allows in the same overlay. The component is not applied globally.

An application namespace whose ServiceMonitor, PodMonitor or PrometheusRule
should be scraped or loaded must carry `app.kubernetes.io/part-of:
baukit-platform`. Without that label Prometheus ignores the namespace. The
shared `deploy/observability` rule and dashboard pack installs into
`observability` and needs nothing extra.

## Validation

Install `kustomize`, `kubeconform`, and `helm`, then run:

```sh
deploy/platform/validate.sh
```

The script discovers every first-level directory containing
`kustomization.yaml`, renders it, validates known Kubernetes resources, and
renders each Flux HelmRelease from its pinned chart, then runs the Flux
convention policies in `lint/ci-policies/` and prints a pass/fail table per
policy. Charts and downloaded core
Kubernetes schemas are cached in `deploy/platform/.helm-cache/`; a warm cache
makes validation offline friendly. `make ci` runs the same validation.

## Adding a base

Create a first-level directory with `kustomization.yaml`, a Namespace, a local
OCIRepository (or a HelmRepository with a reason in the chart sources table) and
HelmRelease manifests (or reviewed plain
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
`rancher/k3s:v1.34.13-k3s1@sha256:fb940fd33b04fe6894e5c09ca1d976e25a5eb378e2ee49144401d660dd1d306f`. The GitOps repository must already contain the
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
| `PLATFORM_K3S_IMAGE` | `rancher/k3s:v1.34.13-k3s1@sha256:fb940fd33b04fe6894e5c09ca1d976e25a5eb378e2ee49144401d660dd1d306f` | k3s image used when creating the cluster. The default is `platform_k3s_image` in `local-git-http.sh`, shared with `platform-up.sh` and the k3d smoke. |
| `PLATFORM_FLUX_VERSION` | `flux` in `deploy/tools.toml` (`v2.9.4`) | Flux pin; override only as a reviewed platform upgrade. |
| `PLATFORM_SNAPSHOT_PORT` | `9418` | Port for local smart HTTP, published on the k3d network gateway. |
| `PLATFORM_API_PORT` | `random` | Host port of the Kubernetes API, bound to `127.0.0.1`. |
| `PLATFORM_HTTP_PORT` | `80` | Host port for HTTP ingress, bound to `127.0.0.1`. |
| `PLATFORM_HTTPS_PORT` | `443` | Host port for HTTPS ingress, bound to `127.0.0.1`. |
| `PLATFORM_STATE_DIR` | XDG state path | Disposable snapshot repository state. |

GitHub mode uses the owner/repository fields to bootstrap and verifies desired
state through the configured branch and path. The URL should be the matching
SSH URL used by the committed `GitRepository`, such as
`ssh://git@github.com/ORGANIZATION/REPOSITORY`. SSH-key mode passes the generic
URL and key directly to `flux bootstrap git`.

In `local-snapshot` mode, the script copies the files Git would commit (from
`git ls-files --cached --others --exclude-standard`, so gitignored files such
as `.env` never enter the snapshot), makes a disposable commit, and serves its
bare repository through `git http-backend` in a named Docker container. The
container runs as uid 65532 with a read-only root filesystem and no
capabilities, and publishes its port only on the k3d network gateway, the
address pods reach as `host.k3d.internal`. The Kubernetes API and ports 80 and
443 bind `127.0.0.1`. Commit metadata is fixed so identical working-tree content retains
the same snapshot revision across repeated `up` calls. The GitOps overlay
should point `GitRepository/baukit` at
`http://host.k3d.internal:9418/baukit.git` on branch `main`. Dumb HTTP is not
supported because Flux's go-git client requires the smart protocol. In
`github-tag` mode no snapshot container is created; the overlay should use the
real Baukit repository and a reviewed `v*` tag.

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
  Kustomizations, HelmReleases, non-ready pods, and where the secrets live.
  It never prints secret values.

Because `unless-stopped` is enforced after both create and start, Docker or
host restarts bring the cluster and snapshot service back without a lifecycle
command. An intentional `down` remains stopped until `up`, as Docker preserves
the user's explicit stop across daemon restarts.

The lifecycle also owns the `host.k3d.internal.server` entry in k3s's
`kube-system/coredns-custom` ConfigMap. k3d's generated `NodeHosts` gateway
entry can disappear after a container/host restart on some releases; the
custom server entry persists in the Kubernetes datastore so Flux can reacquire
the smart-HTTP snapshot without a repair command.

### Flux multi-tenancy lockdown

Both `platform-up.sh` and the lifecycle run source-, kustomize-, helm- and notification-controller with
the patches in `components/flux-lockdown`, then `flux_lockdown_verify` (`flux-lockdown.sh`) checks the
live Deployments and fails `up` when a flag is missing:

| Controller | Flags |
|---|---|
| kustomize-controller | `--no-cross-namespace-refs=true`, `--no-remote-bases=true`, `--default-service-account=default` |
| helm-controller | `--no-cross-namespace-refs=true` |
| notification-controller | none |

`flux bootstrap` takes its manifests from the GitOps repository, so that repository's
`<PLATFORM_GITOPS_PATH>/flux-system/kustomization.yaml` must carry the same patches inline. A remote
component does not work there, because `--no-remote-bases` refuses it. The last patch gives the
bootstrap Kustomization `serviceAccountName: kustomize-controller`; the verifier checks that too.

The component's `images:` list pins each controller of Flux v2.9.4 by multi-arch index digest, and
`flux_lockdown_images` in `flux-lockdown.sh` lists the same references. The GitOps repository's
`flux-system/kustomization.yaml` needs that `images:` list as well: the verifier fails `up` and the
lifecycle bootstrap with `<controller> image <ref>` when a live controller runs another image. A Flux
upgrade changes both lists; `scripts/test_platform_flux_lockdown.py` fails when they disagree.

With `--default-service-account=default`, a Kustomization without `serviceAccountName` runs as the
`default` ServiceAccount of its namespace, which has no RBAC. Platform Kustomizations live in
`flux-system` and set `serviceAccountName: kustomize-controller` (cluster-admin through Flux's
`cluster-reconciler-flux-system` binding). Tenants use `components/tenant`.

notification-controller runs without `--no-cross-namespace-refs`, although Flux documents the flag for
it. The platform Alert in `flux-system` (`observability-stack/flux-notifications.yaml`) watches
Kustomizations, HelmReleases and GitRepositories in every platform namespace, and the flag would make
it ignore all of them. The Kyverno policy `flux-tenant-alert-sources` denies a tenant Alert whose
`eventSources` name another namespace, so a tenant cannot subscribe to platform events.

helm-controller keeps reconciling as itself. Platform HelmReleases live in their component namespaces,
and `restrict-cluster-admin-binding` only lets cluster-admin go to service accounts in `flux-system`
and `kube-system`, so a per-namespace reconciler account is not possible. Tenant HelmReleases get
`serviceAccountName` from `components/tenant`, and Kyverno denies one without it. HelmReleases that
use the `baukit` GitRepository live in `flux-system` with `targetNamespace` and `storageNamespace`.

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
on `127.0.0.1` ports 80 and 443 through k3s ServiceLB, installs Flux, and lets the overlay's Flux
Kustomizations reconcile all pinned HelmReleases. The local flavor includes the
baseline, cert-manager and a self-signed `ClusterIssuer`, Traefik, CNPG and one
PostgreSQL cluster, the observability stack, and RustFS for Loki logs and
PostgreSQL backups. It runs no Keycloak, because production has none: the
fixture verifies tokens from its own compose Keycloak on the host, which
`make dev` in the fixture starts on port 8091. The hcloud base is
intentionally omitted because Docker-hosted k3d has neither Hetzner metadata nor cloud
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

`status` prints where the generated Grafana, RustFS and database credentials
live (the Kubernetes Secret and `secrets.env`), never the values. The state
directory is mode 0700 and its `secrets.env` is mode 0600.

| Variable | Default | Meaning |
|---|---|---|
| `BAUKIT_PLATFORM_CLUSTER` | `baukit-local` | k3d cluster name. |
| `BAUKIT_PLATFORM_STATE_DIR` | `deploy/platform/.local-state` | Generated credentials and snapshot state. |
| `BAUKIT_PLATFORM_GIT_PORT` | `9418` | Snapshot server port on the k3d network gateway. |
| `BAUKIT_PLATFORM_API_PORT` | `random` | Kubernetes API port on `127.0.0.1`. |
| `BAUKIT_PLATFORM_HTTP_PORT` | `80` | HTTP ingress port on `127.0.0.1`. |
| `BAUKIT_PLATFORM_HTTPS_PORT` | `443` | HTTPS ingress port on `127.0.0.1`. | No Secret manifest or credential value is stored in
the publishable overlay. Kubernetes Secrets for PostgreSQL, Grafana,
RustFS, and the initial product are generated and applied at runtime.

### Integration-harness GitRepository contract

Flux still consumes the normal `GitRepository/baukit` contract. The helper
creates a bare snapshot of the files Git would commit from the current checkout
(`git ls-files --cached --others --exclude-standard`): uncommitted and untracked
work is included, gitignored files are not. A small read-only smart-HTTP
server backed by `git http-backend` exposes that snapshot on port 9418 of the
k3d network gateway, after the cluster exists. It rejects requests without a
valid `Content-Length` or larger than 16 MiB. The cluster reaches
it at `http://host.k3d.internal:9418/baukit.git`. This keeps every platform
component on the same Flux-native path used by real overlays and avoids either
publishing a private checkout or bypassing Flux with direct Helm installs.

The local overlay remains offline-buildable: it contains only Flux source and
Kustomization objects, while nested paths and chart fetching are handled at
runtime. `deploy/platform/validate.sh` separately renders every local component
to catch patch and schema errors without needing the Git server or cluster.

### Backups and integration fixture

RustFS runs in the `postgres` namespace with a generated root credential and a
2 GiB local-path PVC. RustFS 1.0.1 uses a multi-platform digest pin and the
image's non-root UID 10001. The AWS CLI bootstrap Job creates `baukit-local`
and `posthog` without replacing existing buckets. The S3 endpoint is
`http://rustfs.postgres.svc.cluster.local:9000`, with region `us-east-1` and
path-style addressing. The CNPG `ObjectStore` writes under
`s3://baukit-local/postgres`; local Loki stores chunks and ruler data in
`baukit-local`. The optional PostHog base uses `posthog` through the chart's
external object storage values. Its bundled object store stays disabled by the
pinned chart's default.

The runtime `rustfs-root` Secrets retain `username` and `password` keys in
`postgres` and `observability`. `postgres-backup-credentials` retains
`ACCESS_KEY_ID` and `ACCESS_SECRET_KEY`. PostHog receives the same credentials
in `posthog-object-storage` with the chart-required `root-user` and
`root-password` keys. Existing secret state gains RustFS credentials on the
next `up` without rotating the other service passwords.

Recreate an existing disposable integration cluster for this storage change.
RustFS gets a new PVC, and the local Loki overlay starts with an S3 schema.
This harness does not migrate old object-store files or filesystem log data.
The publishable Loki base still uses filesystem storage; persistent consumers
must add a future schema period when moving existing logs to S3.

The monthly restore CronJob is enabled locally so it can be exercised
immediately with:

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
PostgreSQL, the host's compose Keycloak, and Tempo. See `local-footprint.md`
for the measured complete-stack footprint.

### Local k3s compatibility and troubleshooting

The infrastructure pin is the stable k3s v1.36 channel. k3s v1.36 rejects
cgroup v1 hosts, including the WSL2 Docker environment used for this proof, so
the k3d flavor uses the newest k3s release that still supports that host:
`v1.34.13-k3s1`. Real nodes must use the I0 v1.36 stable pin. Upgrade the local
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
- If RustFS bootstrap fails, inspect `job/rustfs-create-bucket` in `postgres` and
  confirm the runtime `rustfs-root` Secret exists. The AWS CLI container uses
  writable `/tmp/aws-config` and `/tmp/aws-credentials` paths because it runs
  non-root. The Job has a five-minute deadline and reports bucket errors.
- Use `kubectl get backup,scheduledbackup -n postgres` and the restore Job logs
  for backup problems. A successful restore publishes
  `baukit_restore_test_last_success_timestamp_seconds` through the local
  Pushgateway.
