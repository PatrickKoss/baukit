# Baukit implementation task list

**Source:** [shared-application-platform-analysis.md](./shared-application-platform-analysis.md) (sections 7, 8, 10, 14, 15)
**Orchestration:** Claude Code orchestrator delegating implementation to Codex subagents (`gpt-5.6-sol`, high reasoning), up to 3 in parallel.
**Status legend:** `[ ]` todo · `[~]` in progress · `[x]` done

This document is the single source of truth for what has been done and what remains.
The orchestrator updates it after every wave. Phases 0–2 (Focus A–H) are complete;
their full task lists, friction lists, and wave logs are preserved verbatim in
[implementation-tasks-archive-phase0-2.md](./implementation-tasks-archive-phase0-2.md).

## Completed: Phases 0–2 (Focus A–H, 2026-08-08 → 2026-08-09)

| Focus | Delivered | Exit evidence |
|---|---|---|
| A | baukit built in this repo: 8 Rust crates (runtime/config/core/http/ops/telemetry/openapi/test), 6 TS packages, shared Helm chart, observability pack, CLI + backend/mobile/web templates, release train, agent skills | tag `baukit-v0.1.0`, hosted CI green (the last hosted run before Actions died) |
| B | Fitness Tracker migrated onto baukit (contract migration, B0–B3) | full gate green incl. Docker, branch merged |
| C | OpenDialog migrated (duplicate HTTP metrics removed, Figment → baukit-config, analytics-core) | verified on open-dialog `main` |
| D | solo-leveling-system migrated (standard metric names, envelope, api-runtime); D3 root-caused the utoipa `ApiPath`/`ApiQuery` param regression | full matrix green, merged to main |
| E | Phase 2 gaps: `baukit-auth` OIDC port + mock-OIDC test kit, `--auth oidc` templates + Keycloak compose, friction batch | tag `baukit-v0.2.0`; hosted Actions refused account-wide from here on (billing) — all later gates local |
| F | Reference product `leitbild` (backend+web+mobile: guided program, entries, AI reflection, reminders/streaks) | **Phase 2 exit met: authenticated CRUD app deployed in 5m31.8s** (< 1 h target), leitbild `docs/phase2-exit-run.md` |
| G | architecture-health-platform full rewrite on baukit (backend+web+worker, tree-sitter analysis engine, scoring, `ahp` CLI + GitHub Action, PR governance) | k3d deploy + shutdown/lease-recovery + restore rehearsal green; `baukit-rewrite` is the release branch |
| H | 0.3.0 platform-fix batch from the F/G friction backlog + extraction review (§17.19): `baukit-jobs`, `@baukit/auth-web`, `@baukit/data-contracts-expo-sqlite`, chart/observability/CLI/template fixes, `--worker` generator | tag `baukit-v0.3.0` (10 crates + CLI + 8 packages + 2 charts, coherence-enforced) |

Standing facts that govern all remaining work:

- **No hosted CI.** GitHub Actions refuses jobs account-wide (billing; user will not
  pay). Every "gate green" means the full CI-equivalent run locally, Docker-gated
  tests always included via `--include-ignored`.
- Products (leitbild, architecture-health-platform, and the three migrated apps)
  are still pinned to `baukit-v0.2.0`; leitbild moves to 0.3.0 inside Focus I,
  the rest is later product work.
- Latest release-train tag: `baukit-v0.3.0`.

## Focus I: Phase 3 — cheap production platform (roadmap §15 Phase 3)

Architecture decisions for this focus (recorded as analysis §17 entry 20):

1. **Cluster inventory:** `local` (k3d on the dev machine — k3s-in-Docker, disposable),
   `testing` (small Hetzner k3s node), `production` (Hetzner k3s node per §7.2).
   All single-node and deliberately non-HA. PostHog stays off-cluster on its own
   dedicated server (decision 10 stands).
2. **Two-layer GitOps split:** generic, secret-free platform component bases live in
   baukit under `deploy/platform/` (versioned by the release train, publishable at
   the Phase 4 go-public decision). Everything with identity — domains, ACME e-mail,
   cloud/DNS credentials, SOPS secrets, cluster inventory, app registrations,
   terraform state — lives in a new private repo `github.com/patrickkoss/platform-infra`
   (analysis §14) that is never open-sourced.
3. **Component menu, not a distribution:** each base (cert-manager, Traefik, kyverno,
   CNPG, Keycloak, observability stack, kured, external-dns) is an independently
   includable Flux-native kustomize base; a cluster overlay composes exactly the
   subset it wants. "Optional" means a base you don't reference, never a flag
   inside a shared base.
4. **Flux CD** reconciles all three clusters; SOPS+age with per-cluster keys; baukit
   bases are consumed via a `GitRepository` pinned to a `baukit-v*` release tag
   (read-only deploy key), so platform upgrades ride the existing release train
   and Renovate bumps the tag.
5. **App manifests live in the app repo** (`deploy/gitops/<env>/`: HelmRelease on the
   baukit-app chart + env values + pinned image tag). platform-infra only
   *registers* an app: one read-only `GitRepository` + one Flux `Kustomization`
   per environment. **Promotion is git:** the local release script builds/pushes
   images and bumps the testing tag in the same commit; production is promoted by
   a PR that bumps the production dir (script-assisted). Kargo and Flux
   image-automation are explicitly deferred — revisit at ≥3 deployed apps or when
   PR promotion produces real friction (I10).
6. **Renovate:** the hosted Mend GitHub App on all private repos — it understands
   Flux HelmReleases, Helm, Docker, Cargo git tags, and pnpm, which Dependabot
   does not. No self-hosting. Renovate PRs are validated locally before merge,
   per the no-hosted-CI rule.
7. **DNS records are terraform-managed** initially (few, stable records); external-dns
   exists only as an optional base for later. **Registry:** GHCR with pull secrets
   and a tag-pruning policy; private-storage quota is watched, not assumed.

Order: I0 → I1 (3 ∥, disjoint dirs) → I2 → I3 (2 ∥, disjoint areas) → local pivot (L1 ∥ LD → L1b → L2 → L3, all done) → **M1 ∥ M2 → M3 → M4 → M5** (everything locally implementable). Wave R (all real-environment work) is parked until an explicit user ask. I10 is an optional backlog, not exit-blocking. Former I4/I5b/I6/I7/I8/I9 were dissolved 2026-08-09: locally-testable items moved into M1–M5, real-environment items into Wave R.

### Wave I0 — perishable-claim verification + component picks (1 agent, read/web only)

- [x] Verify current upstream state before anything is pinned: Flux 2.x current minor + bootstrap recommendation (and OCIRepository maturity as a later alternative to git-sourced bases); k3s current stable channel + the supported way to disable bundled Traefik in favor of a Flux-managed one; hcloud-cloud-controller-manager + hcloud-csi-driver current versions and post-June-2026 Load Balancer/Volume prices (decide: Hetzner LB in front of Traefik vs hostPort-only on the single node); kube-prometheus-stack vs individual charts as the Prometheus/Grafana base; **Keycloak install mechanism** (official Keycloak Operator vs maintained chart — Bitnami's 2025 catalog restrictions likely rule bitnami out); CloudNativePG current version and barman-cloud/plugin backup mechanics; Loki + Tempo monolithic-mode chart guidance; hosted Renovate free tier on private personal repos; GHCR private-storage quota + retention/pruning options; Kargo maturity snapshot (evidence for the deferral record, not adoption)
- [x] Append material deltas as analysis §17 decisions (as E0 did); pick the concrete chart/operator + version for every I1 base and record the pin list in this file under I1

### Wave I1 — platform component bases in `deploy/platform/` (3 ∥ agents, disjoint dirs)

Bases are Flux-native kustomize directories (HelmRepository/HelmRelease + pinned
version + opinionated default values + plain-manifest resources where needed),
secret-free and personal-data-free — anything with identity belongs in the
platform-infra overlay. Every base must pass a new `deploy/platform/validate.sh`
(kustomize build + helm template against the pinned chart + kubeconform,
offline-friendly), which becomes part of `make ci`.

#### I0 pin list (verified 2026-08-09)

| Component | Mechanism (chart/operator + repo) | Pinned version | Notes |
|---|---|---|---|
| Flux | CLI bootstrap and controller manifests from [`fluxcd/flux2`](https://github.com/fluxcd/flux2/releases/tag/v2.9.4) | `v2.9.4` | Bootstrap with the [Flux CLI's provider bootstrap](https://fluxcd.io/flux/installation/bootstrap/). Keep baukit bases Git-sourced for release-train reviewability; `OCIRepository` is already GA at `source.toolkit.fluxcd.io/v1`, so OCI is a viable later transport rather than a maturity-blocked one ([API docs](https://fluxcd.io/flux/components/source/api/v1/)). |
| k3s channel | [`stable` update channel](https://update.k3s.io/v1-release/channels/stable) | `v1.36.3+k3s1` | Put `disable: [traefik]` in `/etc/rancher/k3s/config.yaml` (the YAML equivalent of `--disable=traefik`) on every server; see the [configuration](https://docs.k3s.io/installation/configuration) and [networking](https://docs.k3s.io/networking/networking-services) docs. |
| cluster-baseline | Plain Kustomize resources in baukit | n/a | No upstream artifact: namespaces, policies, quotas, and priority classes are repository-owned. |
| cert-manager | `cert-manager` Helm chart from [`https://charts.jetstack.io`](https://charts.jetstack.io/index.yaml) | `v1.21.1` | Install and own the CRDs; issuers remain overlay-owned ([release](https://github.com/cert-manager/cert-manager/releases/tag/v1.21.1)). |
| Traefik | `traefik` Helm chart from [`https://traefik.github.io/charts`](https://traefik.github.io/charts/index.yaml) | `41.2.0` (app `v3.7.10`) | Single-replica/single-node DaemonSet on host ports 80/443; no `LoadBalancer` Service in the initial Hetzner overlays. |
| kyverno | `kyverno` Helm chart from [`https://kyverno.github.io/kyverno`](https://kyverno.github.io/kyverno/index.yaml) | `3.8.2` (app `v1.18.2`) | Latest stable chart; do not pin the newer `3.9.0-rc.1` pre-release ([app release](https://github.com/kyverno/kyverno/releases/tag/v1.18.2)). |
| kured | `kured` Helm chart from [`https://kubereboot.github.io/charts`](https://kubereboot.github.io/charts/index.yaml) | `6.1.0` (app `1.23.0`) | Optional base ([app release](https://github.com/kubereboot/kured/releases/tag/1.23.0)). |
| hcloud CCM | `hcloud-cloud-controller-manager` Helm chart from [`https://charts.hetzner.cloud`](https://charts.hetzner.cloud/index.yaml) | `1.34.0` (app `v1.34.0`) | Keep as an optional provider base for a future managed Load Balancer; it is not composed into the initial single-node overlays ([release](https://github.com/hetznercloud/hcloud-cloud-controller-manager/releases/tag/v1.34.0)). |
| hcloud CSI | `hcloud-csi` Helm chart from [`https://charts.hetzner.cloud`](https://charts.hetzner.cloud/index.yaml) | `2.22.1` (app `v2.22.1`) | Provider base for Hetzner Volume-backed PVCs ([release](https://github.com/hetznercloud/csi-driver/releases/tag/v2.22.1)). |
| CNPG operator | `cloudnative-pg` Helm chart from [`https://cloudnative-pg.github.io/charts`](https://cloudnative-pg.github.io/charts/index.yaml) | `0.29.0` (app `1.30.0`) | Current operator release is [`v1.30.0`](https://github.com/cloudnative-pg/cloudnative-pg/releases/tag/v1.30.0). |
| postgres-cluster | CNPG `postgresql.cnpg.io/v1` `Cluster`/`ScheduledBackup` plus the [`plugin-barman-cloud`](https://github.com/cloudnative-pg/plugin-barman-cloud/releases/tag/v0.14.0) manifests | CRD `v1`; plugin `v0.14.0` | Use a `barmancloud.cnpg.io/v1` `ObjectStore`, Cluster `.spec.plugins` with `isWALArchiver: true`, and `ScheduledBackup.spec.method: plugin`; do not build new configuration on deprecated in-tree `barmanObjectStore` ([plugin concepts](https://cloudnative-pg.io/plugin-barman-cloud/docs/concepts/)). |
| Keycloak | Official namespace-scoped Operator, Kustomize resources from [`keycloak-k8s-resources`](https://github.com/keycloak/keycloak-k8s-resources/tree/26.7.1/kubernetes) | `26.7.1` | OLM-less install via the [official installation procedure](https://www.keycloak.org/operator/installation). Use `KeycloakRealmImport` only for initial realm creation; it does not reconcile updates or deletions ([realm import docs](https://www.keycloak.org/operator/realm-import)). |
| kube-prometheus-stack | `kube-prometheus-stack` Helm chart from [`https://prometheus-community.github.io/helm-charts`](https://prometheus-community.github.io/helm-charts/index.yaml) | `88.2.0` (app `v0.93.0`) | One coherent Operator/Prometheus/Alertmanager/Grafana/node-exporter/kube-state-metrics base; disable unused monitors and HA/Thanos features, use one replica, and bound resources/retention ([chart README](https://github.com/prometheus-community/helm-charts/tree/main/charts/kube-prometheus-stack)). |
| Loki | `loki` Helm chart from [`https://grafana-community.github.io/helm-charts`](https://grafana-community.github.io/helm-charts/index.yaml) | `18.7.6` (app `3.7.6`) | Use `deploymentMode: Monolithic`, `singleBinary.replicas: 1`, and `loki.commonConfig.replication_factor: 1`; the chart moved from Grafana's old repo and chart 12+ renames the mode to `Monolithic` ([guide](https://grafana.com/docs/loki/latest/setup/install/helm/install-monolithic/)). |
| Tempo | `tempo` Helm chart from [`https://grafana-community.github.io/helm-charts`](https://grafana-community.github.io/helm-charts/index.yaml) | `2.2.3` (app `2.10.7`) | The `tempo` chart is the monolithic chart; use `replicas: 1` (not `tempo-distributed`) and configure `tempo.storage.trace` in overlays ([chart guidance](https://grafana.com/docs/tempo/latest/set-up-for-tracing/setup-tempo/deploy/kubernetes/helm-chart/)). |
| Alloy | `alloy` Helm chart from [`https://grafana.github.io/helm-charts`](https://grafana.github.io/helm-charts/index.yaml) | `1.11.1` (app `v1.18.1`) | Logs collector with `controller.type: daemonset`; unlike Loki and Tempo, Alloy remains in Grafana's chart repository ([chart source](https://github.com/grafana/alloy/tree/main/operations/helm/charts/alloy)). |
| external-dns | `external-dns` Helm chart from [`https://kubernetes-sigs.github.io/external-dns`](https://kubernetes-sigs.github.io/external-dns/index.yaml) | `1.21.1` (app `0.21.0`) | Optional, inactive initially because DNS remains OpenTofu-managed ([chart source](https://github.com/kubernetes-sigs/external-dns/tree/master/charts/external-dns)). |

- [x] I1a core + security (`deploy/platform/{cluster-baseline,cert-manager,traefik,kyverno,kured}/`): cluster-baseline (namespace conventions, default-deny NetworkPolicy templates, resource quotas, priority classes per §7.2/7.6); cert-manager (CRDs + controller; ClusterIssuer left to overlays); Traefik HelmRelease with §7.6 defaults (rate-limit/connection-limit middleware, ops listeners never exposed) replacing the k3s-bundled one (disable procedure documented); kyverno + a small baseline policy pack (require requests/limits, disallow `:latest`, block public ops-port exposure) as an optional base; kured as an optional base for coordinated reboots; `hcloud/` optional provider-specific base (hcloud-cloud-controller-manager for `Service type=LoadBalancer` + hcloud-csi-driver for PVC-backed Hetzner Volumes; API token supplied by overlay — generic for any Hetzner adopter, skipped by the local flavor)
- [x] I1b data + identity (`deploy/platform/{cnpg,postgres-cluster,keycloak}/`): CNPG operator base; a reusable `postgres-cluster` component (Cluster with barman-cloud-plugin object-store backups + scheduled backup, per-product database/role convention per §5.3, S3 endpoint/credentials left to overlay); restore-test CronJob skeleton emitting a freshness metric with a staleness alert hook (§5.3 monthly restore test); Keycloak base per the I0 mechanism, backed by the CNPG cluster, realm-as-code convention (one realm per product, import mechanism chosen at I0), admin bootstrap secret supplied by overlay
- [x] I1c observability stack (`deploy/platform/observability-stack/` + existing `deploy/observability` chart): kube-prometheus-stack (or the I0-chosen equivalent: Prometheus + operator + Grafana + kube-state-metrics + node-exporter + Alertmanager), Loki monolithic, Tempo monolithic, Alloy DaemonSet for logs (+ optional OTLP gateway role per §8.3 — one owner per signal, documented); explicit §8.5 retention budgets (30 d metrics / 14 d logs / 7 d traces) and per-component resource limits as first-class values; S3 wiring left to overlay; the existing `deploy/observability` chart (dashboards/rules/alerts) wired in as the content layer with `prometheusRule` enabled; Alertmanager receiver/route convention defined (receiver itself is overlay data)
- [x] Wave gate: `validate.sh` green for every base and for a composed "everything" fixture overlay; `check-metric-names.py` still green; version coherence + `make ci` untouched areas green

### Wave I2 — local platform flavor + integration proof (1 agent, deploy/ only)

- [x] `deploy/platform/overlays/local/` + `deploy/platform/platform-up.sh`: idempotent k3d bring-up/tear-down applying cluster-baseline, cert-manager (self-signed issuer), Traefik, CNPG + postgres-cluster, Keycloak, observability stack with local-sized budgets — the platform's own dev environment, documented in `deploy/platform/README.md`
- [x] Integration proof on the local flavor: scaffold a fixture product (`baukit new fixture --backend --web --worker --auth oidc`), deploy it with the **unmodified** baukit-app chart against the platform Keycloak (realm-as-code) and a CNPG-provisioned database/role; headless-PKCE smoke (existing `deploy/smoke/` harness) green; shared dashboards render fixture metrics with zero unresolved names (the H2a fix proven on-cluster); burn-rate + worker alerts loaded
- [x] Record per-component resident memory/CPU from the running local stack → sizing input for the testing node (I3a)

### Wave I3 — platform-infra repo (2 ∥ agents: I3a terraform/node, I3b flux/sops/config)

New private repo `github.com/patrickkoss/platform-infra` per analysis §14; never
open-sourced. Layout: `terraform/`, `clusters/{local,testing,production}/`
(Flux entrypoints), `platform/` (overlays over baukit bases + sources),
`apps/` (app registrations per env), `secrets/` (SOPS only), `runbooks/`.

- [x] I3a terraform/OpenTofu + node baseline: Hetzner servers for testing (sized from I2 measurements) and production (§7.2 class), cloud-init applying the §7.6 floor (SSH key-only, no root login, unattended security upgrades, fail2ban or CrowdSec, k3s install with bundled Traefik disabled), Hetzner cloud firewall (SSH allowlist, 80/443 only), object-storage buckets (CNPG backups; Loki/Tempo later), DNS records for the chosen domain (terraform-managed; provider = wherever the domain's DNS lives), PostHog server defined but not applied until I8; remote-state decision documented (SOPS-encrypted state in repo vs object-store backend); `tofu validate` + plan green without applying
- [x] I3b Flux + secrets + config: cluster entrypoints (`flux-system`, `platform.yaml`, `apps.yaml` Kustomizations per cluster); platform overlays for local/testing/production consuming baukit `deploy/platform` bases via `GitRepository` pinned to the current `baukit-v*` tag with a read-only deploy key; SOPS+age per-cluster keys (private keys generated offline, stored in password manager + offline backup, public keys committed, kustomize-controller decryption wired); cluster-identity values (domains, ACME e-mail + staging/prod ClusterIssuers, Alertmanager receiver, S3 endpoints); Renovate config for platform-infra plus a shared preset in baukit consumed by all repos (flux, helm, docker, cargo git-tags, pnpm managers); runbook skeletons (bootstrap, node rebuild, DB restore, cert/DNS, secret rotation)
- [ ] **User action:** install the hosted Renovate GitHub App on baukit, platform-infra, leitbild, architecture-health-platform (and the three migrated products); confirm free-tier coverage for private personal repos (I0 verified the claim)
- [x] Wave gate: `flux build kustomization` (or kustomize build) green for every cluster entrypoint against local checkouts; kubeconform green; SOPS encrypt/decrypt round-trip proven with a throwaway key; no plaintext secret anywhere in the repo (checked by a pre-commit/CI-equivalent grep)

### Pivot — local-first platform (user decision 2026-08-09)

No paid environments yet: everything runs on the local machine at zero cost.
One local k3s cluster (k3d, per the I2 WSL2 constraint) hosts the full
platform **and** both app environments — `testing` and `production` as
separate namespaces on the same cluster. Images are built locally and
imported into the cluster (no registry). platform-infra is now on GitHub
(`github.com/PatrickKoss/platform-infra`, private, pushed) so the GitOps loop
is real: push → Flux reconciles. Waves I4/I5b/I6–I9 are **parked** (the
Hetzner path stays intact in terraform/; resume when a paid environment is
wanted); Waves L1–L3 below deliver the same outcomes locally.

Requirement carried into the structure: a future iteration adds progressive
delivery — canary deployments wired into the monitoring stack, automated
tests against a canary that receives **zero** customer traffic, then gradual
traffic shifting only while monitoring stays green, with auto-rollback.
L-wave manifests must be shaped so this can be added without restructuring.

### Wave L1 — local platform cluster on real GitOps (1 agent, platform-infra + local machine)

- [x] Persistent local cluster: k3d (k3s v1.34.8-k3s1 per the I2 WSL2 cgroup
  constraint), Flux v2.9.4; `clusters/local/` in platform-infra becomes a real
  Flux entrypoint reconciling from the **GitHub** repo (deploy key / token via
  `gh`); baukit platform bases served from the local checkout via the I2
  smart-HTTP snapshot mechanism, documented + refreshable (swap to the
  `baukit-v0.3.1` GitRepository tag once that release exists)
- [x] Platform overlays sized for local: cluster-baseline, cert-manager with a
  local CA ClusterIssuer, Traefik, CNPG + postgres-cluster with MinIO backups
  (I2-proven), Keycloak, observability stack (the future canary metric
  source); kyverno Audit optional; hcloud + kured skipped
- [x] SOPS live: real age key for the local cluster generated and installed as
  `flux-system/sops-age` out of band (never committed), local secrets
  encrypted to it, kustomize-controller decrypts on-cluster
- [x] Wave gate: all platform Kustomizations reconcile green **from GitHub**;
  `kubectl delete` self-healing proof; a SOPS-decrypted Secret materializes;
  `validate.sh` still green; footprint recorded vs the I2 numbers

### Wave L1b — reproducible cluster lifecycle automation (1 agent; after L1)

User requirement (2026-08-09): the whole local setup must be automation, not a
procedure. Reproducible spin-up/tear-down, workloads come back after a host
restart, and a full nuke must recreate the cluster identically from code. The
generic mechanics are publishable — other baukit users should get the same
local-platform setup — so the generic layer lives in **baukit** (Makefile
target + script + docs), and platform-infra keeps only the identity-specific
wrapper (repo URL, key paths, cluster name). If it works locally, the same
recipe seeds other clusters later.

- [x] Generic lifecycle in baukit (`deploy/platform/` + Makefile): idempotent
  `up` (k3d cluster with a container restart policy that survives host/Docker
  restarts, Flux install, GitOps source wiring, SOPS key secret install from a
  configurable path), `down` (stop without data loss), `nuke` (delete
  everything incl. volumes), `recreate` (nuke + up, converging to the same
  state purely from git — the reproducibility proof); config via a small
  documented file/env contract (GitOps repo URL + path, baukit source mode:
  GitHub tag vs local-checkout snapshot, age key path), no identity baked in
- [x] platform-infra thin wrapper: `make up/down/nuke/recreate` (or scripts/)
  passing its identity config into the baukit layer; L1's snapshot-refresh and
  re-attach helpers folded into this contract instead of standing alone
- [x] Restart survival proven: restart Docker (or the k3d node containers),
  cluster and all platform workloads return Ready without manual action;
  `recreate` proven: nuke → recreate → all Kustomizations reconcile green from
  GitHub, SOPS secrets rematerialize, footprint matches
- [x] Documented for third parties in baukit (`deploy/platform/README.md` or
  `docs/`): prerequisites, config contract, lifecycle commands, how the same
  recipe generalizes to non-local clusters; platform-infra README documents
  the concrete local instance; baukit `make ci` + validate.sh stay green

### Wave LD — progressive-delivery architecture (1 agent ∥ L1, docs only)

- [x] Decision doc `platform-infra/docs/progressive-delivery.md`: pick the
  canary mechanism (Flagger + Traefik provider vs Argo Rollouts vs plain
  Flux) with pinned versions verified live; define the rollout contract —
  canary gets zero customer traffic until (a) automated tests against the
  canary endpoint pass and (b) monitoring-backed metric checks are green,
  then stepwise traffic shifting with auto-rollback on regression
- [x] Concrete manifest-structure requirements handed to L2 (naming, labels,
  service topology, metric queries against kube-prometheus-stack) so the
  controller can be added later without restructuring

### Wave L2 — app environments + promotion mechanics (1 agent, leitbild + platform-infra; after L1b + LD)

- [x] leitbild `deploy/gitops/{testing,production}/`: HelmRelease on the
  baukit-app chart + env values + pinned image tags; namespaces
  `leitbild-testing` / `leitbild-production` on the one local cluster; per-env
  CNPG database/role + Keycloak realm as code; app secrets SOPS-encrypted to
  the local cluster key
- [x] Registry-less delivery: `make release` builds the per-process images
  (api/worker/migrate) locally and imports them into k3d
  (`k3d image import`), bumping the testing tag in `deploy/gitops/testing/`
  in the same commit; `make promote` prepares the production PR bumping
  `deploy/gitops/production/`; deterministic, documented
- [x] platform-infra registers leitbild (GitRepository + one Kustomization per
  env) — the registration pattern documented as the template for every future
  app
- [x] Progressive-delivery-ready structure per the LD requirements (naming,
  labels, service topology, per-env layering) so a canary controller can be
  inserted later without restructuring
- [x] Wave gate: both envs reconcile green; headless-PKCE smoke +
  authenticated CRUD + worker job end-to-end against the **platform**
  Keycloak in the testing namespace; observability harness green; dashboards
  show both envs

### Wave L3 — promotion proven end-to-end (1 agent; after L2)

- [x] Ship a trivial change through the whole loop twice: `make release` →
  Flux deploys to `leitbild-testing`; `make promote` PR → merge → Flux
  deploys to `leitbild-production`; both verified on-cluster
- [x] Failure drills on the local cluster: kill the api pod mid-request and
  the worker mid-job in the production namespace, clean recovery; CNPG backup
  + restore-test green on MinIO; a deliberately fired alert reaches
  Alertmanager
- [x] Runbooks "deploy a new app to the platform" + "promote a release"
  written from what was actually done

### Pivot continuation — locally implementable waves M1–M5 (planned 2026-08-09)

User decisions recorded: baukit commit freeze **lifted** (orchestrator commits/pushes
baukit and cuts releases); GHCR adopted now via the existing gh CLI auth (images must
stay small — private-tier storage is tight; retention = keep the newest 3 versions per
package); PostHog runs **in-cluster** as an optional, easily-toggled deployment with
minimal resources (the dedicated-server plan in architecture decision 1/§10.3 is
superseded — revisit only if in-cluster proves insufficient, see Wave R).

### Wave M1 — baukit-v0.3.1 release train + cluster onto the real tag

- [x] Commit the pending baukit working tree as clean logical commits (platform
  lifecycle layer, chart `testing` fix, alloy multi-env logs, template Dockerfile,
  renovate preset, docs)
- [x] Focus J library fixes that belong in this release: `testing` accepted by the
  Rust environment type end-to-end (chart already allows it — kills leitbild's
  `staging` bridge at the next adoption); 5xx-percentage query idle-safe
  (`or on() vector(0)`) wherever documented/used; `build_info` can carry the real
  source commit (build-arg → env seam in the template Dockerfile)
- [x] Version 0.3.0 → 0.3.1 coherent everywhere (`scripts/check-version-coherence.py`);
  full gate: `make ci`, tests `--include-ignored`, generated fixture (all flavors),
  metric-names lint, cargo deny if dependencies moved
- [x] Tag `baukit-v0.3.1`, push main + tag to `github.com/PatrickKoss/baukit`
- [x] platform-infra: local cluster consumes baukit via the GitHub tag (read-only
  deploy key per the deploy-new-app runbook pattern, `github-tag` source mode)
  instead of the snapshot container; cluster fully green on the tag; snapshot bridge
  kept documented as the dev-iteration option

### Wave M2 — GHCR-backed delivery (replaces k3d image import)

- [x] GHCR wired via the existing gh CLI token, tested non-interactively first; if
  the token lacks package permissions, stop and report exactly what the user must
  grant — never attempt interactive re-auth
- [x] Images minimized and sizes recorded (multi-stage, static/distroless, stripped);
  the three leitbild process images pushed as private packages
- [x] leitbild `make release` pushes `ghcr.io/patrickkoss/leitbild-{api,worker,migrate}:<sha>`
  and bumps pins as before; cluster pulls via a SOPS-encrypted dockerconfig pull
  secret in leitbild's gitops (rotation documented); `imagePullPolicy` off `Never`
- [x] Retention: newest 3 versions per package, enforced by a script wired into
  `make release`; quota usage documented; rollback-target-must-be-retained risk noted
- [x] Prove one full release→testing→promote→merge→production round pulling from
  GHCR; runbooks updated where the flow changed

### Wave M3 — TLS + exposure checks, fully local (self-signed CA)

- [x] Hostname-based TLS via the local CA chain (`local-ca` ClusterIssuer from L1):
  certificates + IngressRoutes for Keycloak, Grafana, and leitbild (testing +
  production hostnames); verified end-to-end with curl against the CA cert
- [x] leitbild served over TLS; PKCE smoke green through the TLS hostname
- [x] Ops listeners verified unreachable through the ingress (external probe, not
  values inspection)
- [x] Dead-man/watchdog proof local: Watchdog alert routed to a small in-cluster
  heartbeat receiver; silence-means-page proven once by breaking the route
- [x] CNPG restore-test staleness alert demonstrably fires when the CronJob is
  suspended, resolves when resumed
- [x] Runbook/docs updates from what was actually done

### Wave M4 — canary live: Flagger on the local cluster

Orchestrator answers to LD's five open questions (binding decisions for this wave):

1. **Thresholds:** API error-rate < 1% and p95 < 500 ms are the rollout-blocking
   metrics; worker failure %, retry rate, and queue age (< 300 s) are observed on the
   canary dashboard but do not block (see 2); 1 m metric interval.
2. **Worker promotability:** the worker is **not** independently canaried — only
   `leitbild-api` gets a Canary resource; worker + migrate roll as normal rolling
   updates on the same pin. Guard: job-payload changes follow expand/contract; a
   release whose worker cannot run against both API versions must not be promoted.
3. **Gate runner:** Flagger `pre-rollout` webhook → flagger-loadtester in the app
   namespace executing the containerized smoke suite (headless PKCE + CRUD + worker
   enqueue) against the canary Service directly, zero customer traffic — the L2
   rollout-gate NetworkPolicy exists for exactly this.
4. **Per-env weights:** testing = fast lane (steps 20/50, 30 s interval, threshold 3);
   production = 1/5/10/25/50, 1 m interval, threshold 5, full metric gating.
5. **Browser-PKCE in the gate:** no — headless PKCE stays the blocking pre-rollout
   check; browser E2E remains an async post-promotion check, never a rollout blocker.

- [x] leitbild adopts `baukit-v0.3.1`; the `staging` bridge dies (environment
  identity `testing` end-to-end)
- [x] Flagger 1.44.0 as a platform base (direct Traefik provider), composed by the
  local overlay; the LD MetricTemplates with the thresholds above
- [x] Canary resource for `leitbild-api` per env (weights above); gate runner wired;
  `leitbild-primary`/`-canary` Services owned by Flagger as designed in LD/L2
- [x] Proof both ways: a good release promotes through the full weight ladder; a
  deliberately bad release (failing gate or metrics) auto-rolls back, with evidence
  that customer traffic was unaffected
- [x] `docs/progressive-delivery.md` + runbooks updated to reflect reality

### Wave M5 — PostHog in-cluster, optional + flag-gated

- [x] `deploy/platform/posthog/` base in baukit: self-hosted PostHog as a small k8s
  deployment (pinned versions, resources sized for near-zero traffic — measured, not
  copied from hobby sizing), secret-free, optional per architecture decision 3 (a
  base you simply don't compose)
- [x] platform-infra local overlay composes it behind an easy on/off (include the
  base + one values/secret file); documented as off-by-default, on for this test
- [x] leitbild analytics: provider-neutral port unchanged; PostHog web transport
  enabled by a per-env flag (testing on, production off initially); events verified
  arriving in the local PostHog; consent/scrubber behavior re-verified live
- [x] Resource footprint measured + enable-later runbook documented

### Wave I5a — leitbild onto baukit-v0.3.0 (1 agent, leitbild repo)

- [x] Bump git deps + vendored TS packages `baukit-v0.2.0` → `baukit-v0.3.0`; adopt `@baukit/auth-web` (replacing product-local `web/src/auth.ts`), `baukit-jobs` for the F5/F6 outbox+worker (replacing the hand-built kernel — the promoted superset shape, so this is mostly deletion), `SqliteRecordStore` where the product-local Expo adapter duplicates it; chart values updated to 0.3.0 chart features (per-process egress/volumes/env, hook cleanup)
- [x] Full local gate green: fmt/clippy `-D warnings`, tests `--include-ignored` (Docker included), OpenAPI drift, web+mobile gates, observability verification harness — now with **zero** unresolved metrics (the `db_pool_acquire_duration_seconds_bucket` gap must be dead)
- [x] Log every migration friction item — this is the first real 0.2.0→0.3.0 fleet upgrade and the friction list is product research (§11.4) — see leitbild `docs/baukit-v0.3.0-migration.md` + Wave I5a log entry

### Wave R — real environment (**parked** — everything that needs paid infra, real domains, or external services; start only on an explicit user ask)

Bundled 2026-08-09 from the dissolved I4/I5b/I6/I7/I8/I9 (their locally-testable
items moved to M1–M5; L1–L3 already covered registration, promotion, drills,
backups, alert delivery, self-healing locally):

- [ ] Terraform apply: Hetzner nodes (testing + production, §7.2 sizing), firewall,
  DNS records, object-storage buckets, remote state; k3s via cloud-init;
  `flux bootstrap` onto the real clusters — executed **from the runbooks**, every
  gap found becomes a runbook/automation fix
- [ ] Real ACME: staging→production certificates on real hostnames; externally
  verified exposure of exactly 80/443 (ops listeners unreachable from the internet);
  kured reboot window exercised
- [ ] External synthetic check on the production URL + external dead-man heartbeat
  (§8.6) armed; silence-means-page proven from outside once
- [ ] **Node rebuild drill on real infra:** destroy the testing node, rebuild from
  terraform + Flux + backups with no manual resource creation; measured wall-clock
  recorded as the documented RTO
- [ ] **Data restore drill with real production data:** restore into testing;
  leitbild serves an authenticated read of restored data; monthly §5.3 routine
  confirmed
- [ ] Chaos-lite repeat on the real production node (pod kills, node reboot)
- [ ] GHCR pull secrets on the real clusters (mechanics proven locally in M2)
- [ ] **Cost:** record actual monthly € (nodes, object storage, IPv4, DNS/egress)
  against the §7.2 envelope; append as an analysis §17 decision
- [ ] Renovate GitHub App installed on baukit/platform-infra/leitbild (user action)
- [ ] Production AI endpoint switched from the dead placeholder to the real
  integration (needs a user credential decision)
- [ ] PostHog scale-up (dedicated server per §10.3) only if in-cluster PostHog (M5)
  proves insufficient under real traffic
- [ ] **Declare the Phase 3 exit criterion met in analysis §15** (rebuild from code
  + restore data within the documented RTO); file remaining friction as the next
  fix-batch backlog

### Wave I10 — optional backlog (not exit-blocking; schedule on demand)

- [ ] Beyla pilot on the **testing** cluster only (§9): DaemonSet with minimum documented capabilities, coverage of uninstrumented traffic, explicit single-owner rule for HTTP metrics/spans, overhead + route-quality comparison vs in-process instrumentation; remove duplicated signals or remove Beyla
- [ ] architecture-health-platform registered + deployed to testing via the I5b pattern (second consumer of `deploy/gitops/` — after which template emission of `deploy/gitops/` by `baukit new` is justified under the two-consumer rule and goes into the next fix batch, fixture matrix gated)
- [ ] Flux image-automation for testing auto-deploys (only if the manual bump in `make release` proves annoying)
- [ ] Kargo evaluation — trigger: ≥3 deployed apps or measured promotion friction; run it on testing first, never as a day-one dependency
- [ ] external-dns optional base activation if terraform-managed records become churn-heavy
- [ ] Self-hosted CI runners (Blacksmith or actions-runner-controller on the cluster) — trigger: the user decides to un-block hosted CI or its local substitute becomes the bottleneck

## Focus RL: Redis-backed identity + IP rate limiting (planned 2026-08-10)

**Motivation (user ask):** Traefik's `rateLimit` middleware is per-instance,
IP-only, and error-prone as the primary control. Replace it as the real limit
with an app-level, Redis-backed limiter in the Rust backend: **identity-scoped
limits are the primary (low) control; IP-scoped limits are the (very high)
safety net**. Traefik's middleware stays only as a coarse edge net. Leitbild is
the reference integration.

Architecture decisions for this focus:

1. **New crate `rust/crates/baukit-ratelimit`** (ports & adapters, small and
   composable like the rest): a `RateLimitStore` port (async check-and-consume
   for a key + quota → allowed/limited + retry-after), a Redis adapter (single
   atomic Lua token-bucket eval — no MULTI read-modify-write races, `redis`
   crate with tokio connection-manager, pinned `=` version), and an in-memory
   adapter for tests/local dev without Redis.
2. **Axum layer in the same crate**, composable with `baukit_http::layers`:
   per request evaluate **identity scope** (key = `baukit_auth::Principal`
   subject from request extensions, applied only when authenticated, low
   quota) and **IP scope** (client IP with trusted-proxy `X-Forwarded-For`
   handling, high quota, always). Limited → 429 through the standard
   `ApiError` envelope + `Retry-After` and `RateLimit-*` headers. Store
   failure → **fail-open by default** (configurable), warn + metric.
3. **Config** through the existing `baukit-config` conventions
   (`<PREFIX>__RATE_LIMIT__…`: redis URL, per-scope rate/burst/enabled, fail
   mode). No new env-var vocabulary in the chart.
4. **One new metric:** `http_rate_limit_decisions_total{scope="identity|ip",
   outcome="allowed|limited|error"}` — registered in the observability pack
   (lint, dashboard panel, alert on sustained `outcome="error"`).
5. **Chart:** optional `redis.enabled` in `baukit-app` — single-instance,
   pinned image, no persistence (rate-limit state is expendable), NetworkPolicy
   consistent with the baseline, service name `<release>-redis`. Products wire
   the URL themselves via their config env (decision 3).
6. **Release:** new crate ⇒ minor bump `0.3.5 → 0.4.0` across the coherence
   surface, tag `baukit-v0.4.0` via `scripts/release-train.sh`.
7. **Leitbild:** git deps `baukit-v0.3.2 → baukit-v0.4.0`, limiter wired with
   identity low / IP very high, chart redis enabled, compose.yaml redis for
   local dev, Traefik middleware kept but raised to coarse-net levels;
   platform-infra cluster pin `baukit-v0.3.5 → baukit-v0.4.0`. Live 429 proof
   on the local k3d cluster.
8. **Platform Redis base** (user addition 2026-08-10): `deploy/platform/redis/`
   in baukit — a secret-free, optional shared Redis component base per
   architecture decision 3 (component menu), for products that want a shared
   platform Redis instead of the per-product chart redis (decision 5; both are
   legitimate, products choose). Composed by platform-infra's **local** overlay
   and proven live on the k3d cluster like every other component. The base only
   becomes consumable from the cluster after the `baukit-v0.4.0` pin bump, so
   authoring happens in RL1, the live proof in RL3.
9. **Redis version** (user decision 2026-08-10): pin `redis:8.10.0-alpine`
   (digest `sha256:978f0e01593e65eed801f2402944efcd936d43b5027e4908a7897baf88ed6241`
   where digests are used) everywhere — chart, platform base, leitbild compose.
10. **Sentinel-aware store** (user addition 2026-08-10): the mode is decided by
    the replica count — one replica ⇒ plain Redis, more than one ⇒ Sentinel.
    In the crate this stays a single knob: `RedisRateLimitStore::connect`
    accepts `redis://…` (direct, unchanged) **and**
    `redis+sentinel://h1:26379,h2:26379,h3:26379/<master-name>`
    (Sentinel-mediated master discovery via the `redis` crate's `sentinel`
    feature, master role, re-resolve on connection failure). Plus an explicit
    `connect_sentinel(sentinels, master_name)` constructor. No new config
    vocabulary — products switch by changing the URL value only. Fail-open
    semantics and the metric contract are unchanged.
11. **Chart HA:** `redis.replicas` (default 1). `1` ⇒ today's Deployment +
    `<release>-redis` service. Odd `>= 3` ⇒ StatefulSet with a redis and a
    sentinel container per pod, headless service for stable peer DNS, sentinel
    service `<release>-redis-sentinel:26379`, master name `mymaster`, quorum
    `floor(n/2)+1`; template `fail`s on `2` or even counts. Still no
    persistence — replication covers pod loss; full restart resets state,
    which is acceptable for rate-limit buckets and documented. NetworkPolicy
    extended: API pods → 6379+26379, redis pods ↔ redis pods for replication
    and sentinel gossip.
12. **Platform HA base:** new sibling base `deploy/platform/redis-ha/`
    (3 replicas + sentinel, same `platform-redis` namespace, same opt-in
    client label, contract
    `redis+sentinel://redis-sentinel.platform-redis.svc:26379/mymaster`).
    platform-infra's local overlay switches `local-redis` to the HA base so
    Sentinel failover is proven live on k3d; the single-instance base stays
    covered by `validate.sh`.
13. **Release + sequencing:** new public API + chart feature ⇒
    `baukit-v0.5.0`; platform-infra pin `baukit-v0.4.0 → baukit-v0.5.0`
    afterwards. Leitbild stays on v0.4.0 single-redis (HA is an option
    products choose). Constraint: leitbild's `render-gitops` renders the
    baukit **working-tree** chart, so RL4's `deploy/` work must not run while
    RL3 verification is in flight; the `rust/` work is safe in parallel
    (leitbild consumes crates via git tag).
14. **Sentinel testing:** `baukit-test` gains `start_redis_sentinel()` —
    a docker network with master + replica + one sentinel (quorum 1 so
    failover is actually exercisable); Docker-gated tests cover
    sentinel-URL connect + quota enforcement and a master-kill failover.

### Wave RL1 — baukit library + platform surface (2 ∥ agents, disjoint dirs)

- [x] RL1a (`rust/` only): `baukit-ratelimit` crate per decisions 1–4 (port,
  Redis + in-memory adapters, axum layer, config options, metric, docs);
  `baukit-test` gains a Redis testcontainers helper mirroring `postgres.rs`;
  unit tests (in-memory, keying, headers, fail-open) + Docker-gated Redis
  integration tests (atomicity under concurrency, TTL, refill, error path);
  workspace + `deny.toml` additions clean
- [x] RL1b (`deploy/` only): chart optional redis per decision 5 (values,
  templates, README, chart examples); observability pack: metric-name lint
  entry, dashboard panel(s) for `http_rate_limit_decisions_total`, alert on
  sustained store errors; `check-metric-names.py` green
- [x] RL1c (baukit `deploy/platform/redis/` + platform-infra only): secret-free
  platform Redis base per decision 8 (namespace, pinned image, resources sized
  for near-zero traffic, NetworkPolicy, README documenting when to choose it
  over chart redis), following the conventions of the sibling bases;
  platform-infra `platform/local/` composes it (Kustomization against the
  baukit GitRepository, live only after the RL3 pin bump); `validate.sh` green
- [x] Orchestrator gate: `make ci`, rust tests `--include-ignored`, cargo deny,
  MSRV 1.95 check, metric lint, helm template smoke, platform-infra
  `validate.sh`

### Wave RL2 — release train baukit-v0.4.0 (1 agent)

- [x] Version bump 0.3.5 → 0.4.0 everywhere `scripts/check-version-coherence.py`
  demands (crates, packages, templates, charts); coherence green
- [x] Full local gate incl. generated-fixture matrix (backend at minimum; web +
  mobile flavors since template versions change) per CLAUDE.md
- [x] `scripts/release-train.sh` → tag `baukit-v0.4.0`, push branch + tag
- [x] Orchestrator gate: re-run coherence `--tag baukit-v0.4.0` + `make ci`

### Wave RL3 — leitbild reference integration + live proof (1 agent)

- [x] Leitbild backend: bump all baukit git deps to `baukit-v0.4.0`; wire the
  limiter (identity low, IP very high — values chosen in leitbild config per
  env); log migration friction 0.3.2→0.4.0 in leitbild docs
- [x] Leitbild deploy: chart `redis.enabled`, `<PREFIX>__RATE_LIMIT__…` env in
  HelmRelease values, compose.yaml redis for local dev, Traefik middleware
  raised to coarse-net levels (kept as outer safety net)
- [x] Leitbild full gate green (`make check`, tests `--include-ignored`,
  render-gitops) + platform-infra pin bump `baukit-v0.4.0` + `validate.sh`
- [ ] Live proof on local k3d: deploy, hammer an authenticated endpoint past the
  identity quota → 429 + `Retry-After` while a second identity stays 200;
  unauthenticated flood trips the IP net; `http_rate_limit_decisions_total`
  visible in Prometheus/dashboard
- [ ] Platform Redis base live on local k3d (decision 8): after the pin bump the
  `local-redis` Kustomization reconciles Ready, redis pod Ready, reachable from
  an allowed namespace and blocked otherwise per its NetworkPolicy
- [ ] Orchestrator gate: independent re-run of leitbild + platform-infra gates,
  live-proof spot check

### Wave RL4 — Sentinel HA option (decisions 10–14, planned 2026-08-10)

- [x] RL4a (`rust/` only; may run ∥ RL3): `RedisRateLimitStore` sentinel
  support per decision 10 (`redis+sentinel://` URL scheme +
  `connect_sentinel`), `baukit-test` `start_redis_sentinel()` helper per
  decision 14, unit tests (URL parsing/validation) + Docker-gated sentinel
  tests (connect, quota, failover), crate README/CHANGELOG; gates: fmt,
  clippy `-D warnings`, tests `--include-ignored`, cargo deny, MSRV 1.95
- [x] RL4b (`deploy/` only; **only after RL3 gates done** per decision 13):
  chart `redis.replicas` + sentinel StatefulSet path per decision 11;
  platform `redis-ha` base per decision 12; helm lint/template both modes,
  metric lint untouched, platform-infra `validate.sh` compatibility
- [~] RL4c: release train `0.4.0 → 0.5.0`, full gate incl. generated-fixture
  matrix, tag `baukit-v0.5.0`, push
- [ ] RL4d: platform-infra `local-redis` → `redis-ha` base + pin bump
  `baukit-v0.5.0`, `validate.sh`; live k3d proof — sentinel failover
  (delete master pod → promoted replica, labeled client keeps working)
- [ ] Orchestrator gate: independent re-run of all RL4 gates + live spot check

## Log

- 2026-08-10: **Wave RL2 done — baukit-v0.4.0 release train (1 codex agent +
  orchestrator).** `scripts/release-train.sh minor` bumped 41 files (11 crates
  incl. new `baukit-ratelimit`, CLI, 8 TS packages, templates/VERSION, both
  charts); six CLI golden trees refreshed for version-bearing generated files.
  Agent gate: full matrix incl. generated fixture in all three flavors
  (backend fmt/clippy/tests/openapi-drift + `--include-ignored`, web
  build/lint/test, mobile tsc/lint/test). Orchestrator re-ran coherence (0.4.0
  coherent), metric lint, `make ci`, workspace tests `--include-ignored` — all
  green. Tagged `baukit-v0.4.0`, pushed branch + tag.

- 2026-08-10: **Wave RL1 done — Redis rate-limiting foundation (3 ∥ codex agents,
  orchestrator-verified).** RL1a: new `baukit-ratelimit` crate — `RateLimitStore`
  port, Redis adapter (one atomic Lua token-bucket `EVAL` using Redis server
  TIME, TTL'd keys), in-memory adapter, axum layer (identity scope off the
  cached `baukit-auth` `Principal` extension at 60/min+10 burst default, IP
  scope with 1-trusted-hop XFF at 6000/min+500 burst default, fail-open
  default), `ApiError::rate_limited` 429 envelope + `Retry-After`/`RateLimit-*`
  headers, `http_rate_limit_decisions_total{scope,outcome}`, `baukit-test`
  Redis testcontainers helper (pinned `8.10.0-alpine` per decision 9),
  Docker-gated concurrency/TTL/refill/layer integration tests; `Principal` now
  cached in request extensions by the auth extractor; `BSL-1.0` (Boost)
  allowlisted in deny.toml for redis→`xxhash-rust`. RL1b: chart opt-in
  `redis.enabled` (`redis:8.10.0-alpine`, 1 replica, no persistence, NetPol
  API-pods→6379 only, service `<release>-redis`), observability pack: metric
  registered in linter, dashboard panels, critical alert on 10m sustained
  `outcome="error"`. RL1c: `deploy/platform/redis/` shared base (ns
  `platform-redis`, digest-pinned 8.10.0-alpine, opt-in NetPol via
  `baukit.dev/redis-client: "true"` ns+pod labels, URL
  `redis://redis.platform-redis.svc:6379`) + platform-infra `local-redis`
  Kustomization (pin still v0.3.5 — live after RL3 pin bump). Redis 7→8.10.0
  correction applied mid-wave (user decision 9). Orchestrator gates all green:
  fmt/clippy/tests `--include-ignored`, cargo deny, MSRV 1.95, coherence (11
  crates), metric lint, `make ci`, `deploy/platform/validate.sh` (13 bases),
  platform-infra `validate.sh`.

- 2026-08-10: **Wave M5 orchestrator re-verification (independent).** All three
  repos clean == origin (baukit `c706176`, platform-infra `427a27a`, leitbild
  `8e9ae2b`); tags `baukit-v0.3.3/4/5` on the remote; cluster source pinned
  `baukit-v0.3.5@sha1:b1f626d9`. Cluster: 23/23 Kustomizations + 14/14
  HelmReleases Ready (incl. `local-posthog` Ready on v0.3.5), both Canaries
  Succeeded, zero unhealthy pods, PostHog 8/8 Running. Secret hygiene: no
  `phc_`/`phx_` material in either repo, `posthog-secrets.enc.yaml` is
  ENC[AES256_GCM]; leitbild `.env.production` explicit `noop`, `.env.testing`
  `posthog` + host only (no key in git). Privacy proof re-checked at the storage
  layer via clickhouse-client: 2 `journal_entry_saved` rows with
  `entry_id="[redacted]"` + context props; zero rows containing the private
  journal text, the raw email, a `journal_text` key, or the denied-consent
  marker `m5-denied-proof`. Gates re-run by orchestrator, all exit 0: baukit
  coherence `--tag baukit-v0.3.5` + metric lint + `make ci` + rust tests
  `--include-ignored`; platform-infra `validate.sh`; leitbild `make check` +
  `render-gitops` + `check-migrations`. Wave M5 confirmed done — all active
  waves complete; only Wave R remains, parked pending an explicit user ask.

- 2026-08-10: **Wave M5 done — optional, flag-gated in-cluster PostHog proven live.**
  Baukit's new secret-free `deploy/platform/posthog/` base uses the PostHog 30.46.0
  chart / 1.43.0 application image plus pinned PostgreSQL 14.1, Redis 6.2.6,
  ZooKeeper 3.8.4, Redpanda 25.1.9, ClickHouse 22.8.21.38, and BusyBox 1.36.1
  images. It keeps only the ingestion/UI path (web, events, combined plugins,
  single-node dependencies); worker/Celery, recordings, feature-flag service,
  Temporal, object storage, email, backups, toolbox, bundled monitoring, MMDB, and
  the ClickHouse operator are excluded from this near-zero-traffic local profile.
  Releases `baukit-v0.3.3`, `v0.3.4`, then `v0.3.5` were cut: v0.3.4 bounded an
  upstream migration dependency probe discovered during rollout; v0.3.5 made the
  PostHog adapter's immutable Git source self-preparing. Final tag commit
  `b1f626d9`; coherence, `make ci`, Docker-gated Rust tests, metric lint, and the
  combined backend/web/mobile generated fixture all pass. platform-infra
  `427a27a` pins v0.3.5 and composes `local-posthog` plus one SOPS values/secret
  file as the obvious toggle (documented off by default, enabled for this proof).
  leitbild `8e9ae2b` leaves its provider-neutral event seam unchanged and selects
  the PostHog web adapter only with `VITE_ANALYTICS_PROVIDER=posthog`; testing is
  on, production is explicit noop, and missing key/host fails closed. The real
  testing singleton emitted a consented proof event through the pinned 1.43.1 web
  client: attempted `entry_id=person@example.com` arrived as `[redacted]`, the
  deliberately forbidden `journal_text` property was absent, and context retained
  `app=leitbild` / `environment=testing`; ClickHouse and the authenticated PostHog
  events API each returned the matching event. A denied-consent event produced no
  row. Three settled idle samples totalled 67–86m CPU and about 1.55 GiB RAM; pod
  ranges and measured storage are recorded in the enable/rotate/disable runbook.
  Requests total 130m CPU / 1.77 GiB, limits 3.25 CPU / 4.38 GiB, and claims 5.5
  GiB. Final live audit: 23/23 Kustomizations and 14/14 HelmReleases Ready, both
  leitbild Canaries Succeeded with zero failed checks after testing 20/50 and
  production 1/5/10/25/50, PostHog health `ok`, and zero unhealthy pods. No
  product image release was required because the current cluster has no web
  workload; the chart metadata rollout still rode both canaries without bypass.
  Gates pass: platform-infra `validate.sh`; leitbild `make check`, web format/build/
  lint/test, `make render-gitops`, and `make check-migrations`. Deviations: the
  release advanced twice for rollout-found fixes; Docker Hub rate limiting uses
  the same-digest `mirror.gcr.io` ClickHouse image; during proof diagnostics an
  initial database password and then a client project key were each exposed once
  in local command output, immediately rotated in the live service and SOPS file,
  with no plaintext committed. Focus J: fix `make up` ordering so a suspended
  parent reconciliation is resumed before waiting for the Baukit source, and
  evaluate a newer lean analytics backend when upgrading beyond legacy PostHog.

- 2026-08-09: **Wave M2 done — GHCR-backed delivery live (codex gpt-5.6-sol, orchestrator-verified).**
  Delivery now flows through GHCR instead of `k3d image import`. Images minimized to
  process-specific scratch runtimes: api 122→22 MB, worker 122→21 MB, migrate
  122→10 MB, rollout-gate 67→61 MB (73.8% total reduction; ~40.6 MB unique
  compressed layer bytes across all four packages; ≤3-version ceiling ≈127 MiB
  compressed). `make release` (scripts/release.sh) logs into ghcr.io via
  `gh auth token`, pushes `ghcr.io/patrickkoss/leitbild-{api,worker,migrate,rollout-gate}:<12-sha>`,
  and runs `scripts/prune-ghcr.sh` (keep-newest-3, refuses non-private packages).
  Pull secret `leitbild-ghcr` (dockerconfigjson) SOPS-encrypted in both env gitops
  dirs; inherited by Flagger-cloned primary/canary Deployments and by both
  loadtesters (HelmRelease post-render patch); all GHCR workloads `IfNotPresent`.
  Smoke-ai deliberately stays node-local `Never` (avoids a fifth private package).
  Proof round `dd2f1a091d38` (build 0.1.3): release 104 s; testing canary walked
  20/50 in 161 s; PR #6 merged `4e31d85`; production walked 1/5/10/25/50 in 519 s
  (merge→completion 684 s); real registry pull events on canary AND promoted
  primary pods in both envs (e.g. api pull 726 ms / 8.8 MB). Synthetic TLS traffic:
  testing 711 requests all expected; production 2,295 with one 504 during primary
  transition (same M4 symptom, Focus J). Retention verified: exactly one version
  per package (`dd2f1a091d38`), migration-bridge `70da…` versions deleted, no probe
  or temp artifacts left in GHCR/cluster/git.
  Orchestrator re-verification: all three repos clean == origin (leitbild `135788b`,
  platform-infra `3f31d87`, baukit untouched); GHCR listed via `gh api` — 4 packages
  × 1 version, tags exactly `dd2f1a091d38`; `git grep` for `gho_`/`ghp_`/`github_pat_`
  clean in both repos; both pull-secret files ENC[AES256_GCM]; cluster 22/22
  Kustomizations + 13/13 HelmReleases Ready, both Canaries Succeeded failedChecks 0,
  zero unhealthy pods; live Deployments show GHCR image refs + `IfNotPresent` +
  `leitbild-ghcr` on api/primary/worker/loadtester in both envs; kubelet `Pulled`
  events confirm real GHCR pulls at walk timestamps; `build_info` from production
  primary reports commit `dd2f1a091d38…` v0.1.3; PR #6 MERGED; only remaining k3d
  imports are smoke-ai + the dev smoke script. Gates re-run by orchestrator: leitbild
  `make check`/`render-gitops`/`check-migrations` and platform-infra `validate.sh`
  all exit 0. Deviations: Flux briefly reconciled the source-bump commit before the
  release commit during the one-time migration, causing pulls of the old tag's new
  GHCR name; agent published exact-byte bridge images to let remediation finish,
  then deleted them (primary stayed serving throughout). Focus J: the recurring
  single 504 during primary template transition; first-class loadtester pull-secret
  seam (currently product-local post-render patch); N/N-1 gate work continues.
  Risks: only `dd2f1a091d38` retained — rollback to older tags requires exact-source
  re-release; minimized runtimes assume amd64 + Ubuntu 24.04-compatible glibc.

- 2026-08-09: **Wave M4 done** (1 codex agent, all three repos) — **canary delivery is live: Flagger 1.44.0 gates every leitbild release in both environments, proven good AND bad**. baukit: `baukit-v0.3.2` released (tag object `16a5d9e1` → commit `c7ab39e`; carries the `78ad746` lifecycle fix — orchestrator confirmed ancestry) with new composable base `deploy/platform/flagger/` + six shared MetricTemplates; commits `be8addc`,`da2938a`,`1d6b27f`,`c7ab39e` (+docs `21760bc`); full release train green (agent: make ci, --include-ignored, six fixture flavors, coherence, metric lint, cargo deny; orchestrator re-ran make ci + --include-ignored + coherence `--tag baukit-v0.3.2` + metric lint + backend fixture flavor — all exit 0; release diff in cli/templates was version-bumps + snapshots only). platform-infra `cfc9d20`,`5db30fc`: Flagger install (CRD-race-safe phases) + pin → `baukit-v0.3.2`; cluster source `baukit-v0.3.2@sha1:c7ab39ee` live. leitbild adopted 0.3.2 (`1cb4569`…`05a8e57` incl. PR #5): **staging bridge DELETED** — `LEITBILD_ENVIRONMENT=testing` live in the testing primary (orchestrator read the Deployment env), and `build_info{commit="70da514c…"}` now reports the real SHA from BOTH envs (orchestrator port-forwarded both primaries); full leitbild CI mirror green (orchestrator re-ran make check + render-gitops + check-migrations, exit 0). Canary wiring exactly per the five binding decisions (orchestrator read the live Canaries): only `leitbild-api`; testing 20/50 @30s threshold 3, production 1/5/10/25/50 @1m threshold 5; blocking metrics error%≤1 + p95≤0.5s @1m; worker templates observe-only; blocking `type: bash` pre-rollout webhook → namespaced loadtester 0.38.0 running the pinned headless-PKCE/CRUD/worker gate image against `leitbild-canary` at weight 0 (zero customer traffic). GOOD run `70da514c0966`: testing walk 164 s, production walk 522 s, Flagger events per phase, promotions clean. BAD run `2c1fa7fd5fd0` (testing only): 3 consecutive gate failures → auto-rollback at threshold, canary weight stayed 0, primary stayed `11ca65c84c8c`, 1,046-request TLS probe with ZERO unexpected responses; bad tag absent from git (orchestrator grepped gitops). Final state (orchestrator re-verified): 22/22 Kustomizations, 13/13 HelmReleases Ready, both Canaries `Succeeded` failedChecks 0, zero unhealthy pods, both envs fully pinned `70da514c0966`, Traefik `leitbild-primary=100`, validate.sh green, all three repos clean == origin/main. Docs: progressive-delivery.md design→reality, promote-release runbook (promotion rides the canary, N/N-1 worker guard), flagger base README. Deviations: loadtester `type: cmd` is async and can't block — live resources use `type: bash` (the immutable v0.3.2 README still shows `cmd`; Focus J to fix next release); production gate checks authenticated enqueue acceptance (no deterministic AI backend there by design); one transient customer-visible 504 per env during primary spec transitions (Focus J: investigate). Further Focus J: chart-native Canary/ServiceMonitor/NetworkPolicy seams; parameterize idle/NaN-safe API templates; formalize + continuously test N/N-1 API-worker contract; decide production worker-completion backend vs enqueue-only.
- 2026-08-09: **Wave M2 unblocked (user action).** The gh CLI's fine-grained PAT could not push GHCR packages (fine-grained PATs are unsupported by the container registry regardless of permissions — verified live via probe push, `permission_denied` on scopes). User switched gh to an OAuth login with `write:packages` + `delete:packages` (+ repo/read:org). Orchestrator verified the full chain live: `docker login ghcr.io` via `gh auth token`, probe image push to `ghcr.io/patrickkoss/baukit-auth-probe:tmp`, and package deletion via `gh api DELETE /user/packages/container/…` — all green; probe package deleted, no leftovers. M2 relaunches after M4 completes (leitbild collision avoidance); token sourcing stays `gh auth token`.
- 2026-08-09: **Wave M3 done** (1 codex agent, platform-infra + leitbild) — **TLS, exposure, dead-man, and staleness checks all proven locally on the self-signed CA**. Hostname scheme: `keycloak.platform.local`, `grafana.platform.local`, `leitbild-testing.local`, `leitbild-production.local` (+ `leitbild.local` compatibility alias — production's create-only Keycloak realm still owns that callback; canonical rename filed under the KeycloakRealmImport Focus J). All four hosts serve HTTPS through Traefik with cert-manager leaves off the `local-ca` chain (root CN `platform-local-root-ca`, valid to 2036); verified from the host with `curl --cacert` + `--resolve` (no `-k`) and `openssl s_client` verify code 0 — orchestrator reproduced all four. PKCE + CRUD smoke green over `https://leitbild-testing.local` trusting only the CA. Ops exposure: Traefik routes only the three customer ports; `/metrics`/`/healthz` 404 via the edge, Grafana ops paths 403 by explicit middleware; host probes to 9090/9091/9093/9100 all connection-refused; serverlb publishes exactly 80/443 + API port (orchestrator re-probed). Dead-man local analogue (§8.6): permanent 2 MiB BusyBox webhook receiver gets the Watchdog every 30 s, ServiceMonitor + `BaukitWatchdogHeartbeatStale` (>180 s for 1 m); silence-means-page proven (route broken → firing critical in Alertmanager at 338 s age → restored → zero active). CNPG restore-test staleness: first attempt honestly aborted after finding Prometheus didn't select rules from `postgres` — permanent fix `ad0769e`, then the drill: CronJob suspended + temporary live-only threshold → `BaukitPostgresRestoreTestStale` firing → permanent rule (35 d, `for: 1h`) + CronJob restored, alert resolved; orchestrator confirmed `suspend=false` and no temp rules remain. Commits: platform-infra `e981d28`,`5c5e8d4`,`ad0769e`,`47d8419`,`f3ba424`; leitbild `7d5d60a`,`8f3c3f9`,`27f7901`; both repos clean == origin. Gates green (orchestrator re-ran): validate.sh (24 dirs/11 SOPS), leitbild render-gitops (33/30) + check-migrations, 20/20 Kustomizations Ready, 7/7 Certificates Ready, zero unhealthy pods. Docs: `local-cluster.md` + new runbook `local-alert-drills.md`. Focus J: chart's ingress-controller NetworkPolicy default assumes `kube-system` but baukit's Traefik base uses `traefik`; baukit observability selects rules only from `observability` while the postgres base emits alerts in `postgres`; KeycloakRealmImport create-only → redirect changes need a real reconciliation mechanism. Note: browser use needs hosts-file entries + manual CA trust (documented).
- 2026-08-09: **Wave M1 done** (1 codex agent, baukit + platform-infra) — **`baukit-v0.3.1` published and the local cluster now consumes the real GitHub tag**. Baukit: 11 clean commits pushed (pending Focus I tree split into logical commits: platform lifecycle layer, chart/monitoring fixes, generated Dockerfiles + pool metrics, renovate preset, docs; then the three Focus J fixes; then the release train), annotated tag `baukit-v0.3.1` → release commit `c292c3c`. Focus J fixes shipped: `DeploymentEnvironment::Testing` end-to-end in baukit-core (+config/telemetry/chart, tests); 5xx ratios idle-safe via `or on() vector(0)` in recording rules/burn-rate/dashboard (promtool 3.13.2 validated 9+13 rules); `GIT_COMMIT` build-arg→env seam in the template Dockerfile feeding the existing `option_env!` `build_info` wiring (fixtures updated). Gates all exit 0 (agent + orchestrator re-runs): `make ci`, `--include-ignored`, version coherence `--tag baukit-v0.3.1`, metric lint, cargo deny, MSRV 1.95, full generated fixture (backend `--include-ignored` + web + mobile). platform-infra `1982a71`: baukit GitRepository → `ssh://git@github.com/PatrickKoss/baukit.git` @ tag `baukit-v0.3.1` with fresh read-only deploy key (id 159740944, private half SOPS-only, temp files shredded; 11 SOPS files, validate.sh green), lifecycle config on `github-tag` mode, snapshot container removed. Orchestrator verified live: source revision `baukit-v0.3.1@sha1:c292c3c`, 18/18 Kustomizations Ready, HelmReleases `0.3.1+c292c3ce5dcb` in both leitbild envs + observability, zero unhealthy pods, snapshot container gone, both repos clean == origin. Deviation (accepted): a one-line tag-mode lifecycle bug (`ensure_snapshot_dns` no-op returned 1 under `set -e`) was found during the swap and fixed as `78ad746` AFTER the tag — the immutable tag lacks that fix; carry `78ad746` into the next release. Leitbild untouched (chart constraint `*` accepted 0.3.1 without changes; dep adoption stays Wave M4).
- 2026-08-09: **Re-plan after L3 (user decisions).** Age key backed up by the user. **baukit commit freeze lifted** — orchestrator authorized to commit/push baukit and cut releases (gh CLI available). Parked waves I4/I5b/I6/I7/I8/I9 dissolved: everything needing paid infra/real domains/external services bundled into **Wave R** (parked until explicit ask); everything locally implementable became **Waves M1–M5** (M1 baukit-v0.3.1 release + cluster onto the real tag; M2 GHCR-backed delivery — small images, keep newest 3 versions; M3 local TLS/exposure/dead-man/staleness checks via the self-signed CA; M4 Flagger canary live — the five LD open questions answered as binding orchestrator decisions in the M4 section; M5 PostHog in-cluster as an optional flag-gated small deployment, superseding the dedicated-server plan). Waves M1 ∥ M2 launched (2 codex agents; M1 owns baukit + platform-infra, M2 owns leitbild + GHCR; narrow overlap on leitbild's chart-version pin coordinated in both prompts).
- 2026-08-09: **Wave L3 done** (1 codex agent, leitbild + platform-infra) — **the promotion loop is proven end-to-end, twice, on real merged PRs**. Round 1: `build_info` 0.1.0→0.1.1, tag `a1d7a751cf28`, [PR #1](https://github.com/PatrickKoss/leitbild/pull/1), release→testing-ready 106 s, merge→production-ready 107 s. Round 2: 0.1.1→0.1.2, tag `ce03721d4fb4`, [PR #2](https://github.com/PatrickKoss/leitbild/pull/2), 65 s / 80 s (explicit `flux reconcile` used to bound timings; deployment still flowed from pushed GitHub commits). Orchestrator re-verified live: both envs' Kustomizations at `64fea8e5`, all deployments (api/worker/smoke-ai) on image tag `ce03721d4fb4`, and `build_info{version="0.1.2"}` served from both namespaces' ops endpoints. **All four failure drills PASS in production/platform namespaces**: api pod deleted mid-request-loop → new pod Ready in 2.45 s, 8 failed requests, 2.4 s error window (expected single-replica outage, documented); worker force-killed mid-job (temporarily slowed via suspend+env swap) → job lease (15 min) expired, replacement worker reclaimed attempt 2, final `succeeded|2`, exactly one reflection, no loss; CNPG on-demand base backup `20260809T153648` (10 s) landed in MinIO + restore-test job recovered a throwaway cluster in 62 s and pushed the freshness metric, throwaway resources removed; temporary `PrometheusRule` (`vector(1)`) shown **firing in Alertmanager** with correct labels, then resolved to zero active after rule deletion. Orchestrator confirmed zero drill leftovers (no l3 pods/rules/jobs, worker env restored to committed values, nothing suspended). Runbooks written from the executed commands in platform-infra `docs/runbooks/{deploy-new-app,promote-release}.md` (registration pattern generalized; rollback = re-promote a known-good tag via `TAG=… make promote`, incl. the rebuild+reimport path after cluster recreation) — commit `4531931` pushed. leitbild: 8 commits pushed (2 bumps, 2 releases, 2 promotes, 2 merges; final `64fea8e`). Gates all green on orchestrator re-runs: 18/18 Kustomizations + 10/10 HelmReleases Ready, zero unhealthy pods, platform-infra validate.sh (22 dirs/10 SOPS files), leitbild `make check` exit 0 + render-gitops (33/30 resources) + check-migrations, both repos clean == origin/main; **baukit untouched by L3** (working tree exactly the pre-existing L1b/L2 state, zero commits). Focus J candidates: configurable baukit-jobs lease duration / fenced reclaim (crash recovery waits 15 min); production-safe worker drill+enqueue harness (no direct SQL / Helm suspension); `make promote` local-branch cleanup + safe reruns; embed source SHA in `build_info` (currently `commit="unknown"`). Risks: node-local images still require rebuild+import after cluster recreation; production AI endpoint intentionally dead until real integration is authorized. **This completes the local wave set L1/L1b/LD/L2/L3 — Phase 3's local pivot is done.**
- 2026-08-09: **Wave L2 done** (1 codex agent, leitbild + platform-infra). Both app environments live as namespaces on the local cluster via **real GitOps**: leitbild `deploy/gitops/{base,testing,production}/` (10 commits pushed, final `a96f316`; first commit = the verified I5a baseline `21d6f53` — leitbild is no longer commit-frozen, the git-based promotion loop requires it), platform-infra registers both envs (`da1ae41`: one GitRepository + one Kustomization per env, read-only deploy key `platform-local-flux-20260809` — private half SOPS-encrypted only, plaintext deleted; pattern documented as the future-app template). **LD 13-point checklist: 12 SATISFIED, 1 DEVIATED** — orchestrator spot-checked live: immutable `baukit.dev/workload=leitbild-api` on selector+template (postRenderer), IngressRoute→TraefikService edge with zero `Ingress` objects, only base Services (`-primary`/`-canary` reserved), clone-safe NetworkPolicies incl. rollout-gate allowance, reserved `progressive-delivery/` dirs, Flagger 1.44.0 CRD + sample Canary fixture validated offline. Deviation (checklist 10): baukit v0.3.0's Rust env enum can't parse `testing` → testing process bridges `LEITBILD_ENVIRONMENT=staging` while ALL external identity (namespace, realm, metrics, logs, chart) is exactly `testing`; enum fix = Focus J. **Registry-less delivery proven**: `make release` = build api/worker/migrate images (tag = 12-char git SHA, `imagePullPolicy: Never`) + `k3d image import` + atomic testing-pin bump commit (release `5c9a929e5bab` deployed); `make promote` preps the production PR via worktree + `gh pr create` (intentionally not run — L3 owns the loop; production pinned directly to the same verified tag). Wave gate green (orchestrator re-ran independently): both env Kustomizations + HelmReleases Ready at `a96f316`, all pods Running, zero unhealthy cluster-wide; **headless-PKCE login + authenticated CRUD + worker job end-to-end (`status=succeeded attempts=1`) in `leitbild-testing` against platform Keycloak** (testing AI = deterministic in-cluster smoke responder, no external API/credential); live Prometheus shows `product/service/environment` exact for both envs + `rollout=canary` relabel active; Loki now multi-env (baukit alloy.yaml derives env from `baukit.dev/environment` — one of exactly 2 uncommitted baukit edits, the other: chart `_helpers.tpl` allows `testing`); render-only readiness gate (33/30 resources) + expand/contract migration gate + platform-infra validate.sh (10 SOPS files) + full leitbild CI mirror (fmt/clippy `-D warnings`/tests `--include-ignored`/web/mobile/observability) + full baukit `make ci` — all exit 0 on orchestrator re-runs; plaintext-key sweep clean. Focus J candidates: `testing` in the Rust env enum; 5xx query needs `or on() vector(0)` for the all-success case; move rollout selector/relabel/policy post-renders into chart capabilities; worker/API release coupling before any live Canary; CNPG-vs-HelmRelease cold-bootstrap ordering; `KeycloakRealmImport` update semantics. Risks: node-local images mean a cluster recreate needs rebuild+reimport of pinned tags; shellcheck unavailable locally (`bash -n` + review only).
- 2026-08-09: **Wave L1b done** (1 codex agent). Cluster lifecycle is now automation with the generic layer in **baukit**: `deploy/platform/platform-lifecycle.sh` + `make platform-{up,down,nuke,recreate,status} PLATFORM_CONFIG=...` — identity-free (config contract: cluster name; GitOps URL/branch/path + auth mode `github`|`ssh-key`; `PLATFORM_AGE_KEY_FILE`; baukit source mode `github-tag`|`local-snapshot` + checkout path; optional k3s image/Flux version/snapshot port/state dir; defaults k3s v1.34.8-k3s1, Flux v2.9.4). platform-infra reduced to a thin wrapper (`make up/down/nuke/recreate/status` → `scripts/platform.sh` supplying only identity; the four standalone L1 helpers deleted, no duplicate logic); 2 commits pushed (final `c9ca4fe`), `docs/local-cluster.md` updated, third-party docs in baukit `deploy/platform/README.md` incl. remote-cluster adaptation note; generated state gitignored (`deploy/platform/.local-state/`). **All three proofs executed live**: restart survival (`docker restart` of server, serverlb, snapshot containers → fully Ready unaided in 275 s; all three verified `restart=unless-stopped` — orchestrator re-checked `docker inspect`); `down`→`up` in 253 s with state intact (kube-system ns UID, SOPS Secret UID, CNPG PVC UID, snapshot revision unchanged); **nuke→recreate in 616 s (10m16s)** from nothing — cluster, volumes, network, snapshot container/image, state dir all gone, then converged green purely from GitHub + the age key file (SOPS Secret new UID, value hash == decrypted git value; footprint in the L1 band: 28 pods, node 5.3 GiB/407m). Gate green (orchestrator re-ran independently): 16/16 Kustomizations + 8/8 HelmReleases Ready at `c9ca4fe6`, zero unhealthy pods; platform-infra validate.sh (22 dirs/3 Flux/3 SOPS builds/9 enc files); full baukit `make ci` exit 0 + `cargo test -- --include-ignored` exit 0 (24 suites, Docker-gated Postgres tests ran); `bash -n` + shellcheck on new scripts; baukit received **zero commits** (working tree only: `Makefile` + `deploy/platform/`, identity grep clean outside gitignored state). Deviations (justified): idempotent `flux bootstrap github` kept for recreate (regenerates private-repo auth after nuke; committed manifests unchanged) instead of `flux install`+apply; k3d loses `host.k3d.internal` on container restart ([k3d#926](https://github.com/k3d-io/k3d/issues/926)) → lifecycle owns a persistent `coredns-custom` gateway record + CoreDNS wait, full proof re-run after the fix. Focus J candidates: swap snapshot bridge → `baukit-v0.3.1` tag; drop the CoreDNS workaround when k3d fixes restart-persistent aliases; several-minute Flux dependency-status republish delay after controller restarts. Existing `platform-up.sh` dev harness kept and documented as separate from the persistent GitOps lifecycle.
- 2026-08-09: **Wave L1 done** (1 codex agent). Persistent local platform cluster `platform-local` (k3d, k3s v1.34.8+k3s1) live on **real GitOps**: `flux bootstrap github` (v2.9.4) onto `clusters/local` of `github.com/PatrickKoss/platform-infra`; 11 commits pushed (final `fc6a8af`). Platform overlay `platform/local/`: cluster-baseline, cert-manager + local CA ClusterIssuer chain (`local-self-signed`→`local-root-ca`→`local-ca`, all Ready), Traefik, CNPG + postgres with MinIO-backed ObjectStore (WAL archiving healthy; restore drill deferred to L3 by design), Keycloak, full observability stack. baukit bases served from the local checkout via a **persistent smart-HTTP snapshot container** (`platform-local-baukit-git`, restart=unless-stopped — deviation from I2's host process, which died with its shell); refresh = `scripts/refresh-baukit-snapshot.sh refresh` + `flux reconcile source git baukit` (propagated fully Ready in 75 s); `platform/local/source.yaml` carries the swap-to-`baukit-v0.3.1` comment. SOPS live with a real age key (`~/.config/sops/age/platform-local.agekey`, 0600, outside all repos), `secrets/local/` encrypted, kustomize-controller decrypts on-cluster (decrypted-git-value SHA-256 == live-Secret SHA-256). Gate green (orchestrator re-verified on-cluster + re-ran validate.sh, exit 0): 16/16 Kustomizations + 8/8 HelmReleases Ready from GitHub, zero unhealthy pods, SOPS Secret materialized, self-healing proof (deleted `Deployment/minio` → Flux restored it unaided in ~5m25s, new UID), validate.sh 22 kustomize dirs / 3 recursive Flux builds / 9 SOPS files. Footprint ≈ I2 (node 6.0 GiB / 236m vs 6.3 GiB / 123m; Keycloak 596 Mi, Prometheus 592 Mi, Grafana 347 Mi). Deviations (justified): kyverno omitted (optional, footprint); two corrective commits wiring `gotk-components/sync.yaml` (I3b's empty bootstrap placeholders blocked auto-reference — Focus J: redesign for testing/production); Keycloak patch target fixed (namespace assigned late by baukit's transformer — Focus J: silent zero-match Kustomize selectors need rendered-manifest assertions). Risks: age key has a single filesystem copy (**user: back it up to the password manager**); MinIO/local-path storage is dev-grade, not DR. baukit tree untouched by the agent. Orchestrator then committed the Wave LD doc (`d52ff69`) after L1 released the repo.
- 2026-08-09: **Wave LD done** (1 codex agent ∥ L1, docs only — ownership held: single file `platform-infra/docs/progressive-delivery.md` (530 lines, 24 live citations), zero git ops, zero cluster ops). **Decision: Flagger 1.44.0** (Flux-family) with the direct Traefik provider (`traefik.io/v1alpha1 TraefikService`; Flagger 1.43+ speaks Traefik v3's API group, verified against Traefik chart 41.2.0) over Argo Rollouts and plain-Flux staging — Flagger targets the HelmRelease-rendered Deployment (helm-controller stays desired-spec owner, drift exclusion is a supported integration), `pre-rollout` webhooks give the zero-customer-traffic test gate, MetricTemplates (six baukit-specific PromQL templates: http error %/p95, worker failure %/p95/retry/queue-age) gate stepwise 1/5/10/25/50% weights with auto-rollback. Deliverables: full rollout state machine incl. expand/contract migration constraint; **13-point manifest-structure checklist handed to L2** (highlights: immutable `baukit.dev/workload` rollout selector separate from stable identity labels; `ingress.enabled: false` + `IngressRoute`→bootstrap-`TraefikService` edge topology committed now with `ssa: IfNotPresent`; reserve `<product>{,-primary,-canary}` Service names; ops ServiceMonitor selects stable labels + adds `rollout=canary|primary` relabeling; clone-safe NetworkPolicies; HPA `autoscalerRef`; render-only readiness gate in CI). 8 Focus J conflicts filed (chart lacks a Flagger-safe selector, standard Ingress bypasses weighted TraefikService, ServiceMonitor can't distinguish tracks, NetworkPolicy blocks the gate runner, API-rollback-vs-worker coupling, irreversible migrations, `deploymentEnvironment` rejects `testing`, Loki single-env-per-cluster assumption). 5 open questions parked for the user (thresholds, worker promotability, gate-runner shape, per-env weights, browser-PKCE canary test). Doc left uncommitted until L1 releases the repo (L1 owns git state this wave).
- 2026-08-09: **Local-first pivot + platform-infra pushed.** User decision: no paid environments yet — the whole platform runs on the local machine (one k3d cluster, `testing`/`production` app envs as namespaces, images built locally + imported, no registry); Hetzner waves I4/I5b/I6–I9 parked (terraform kept dormant); new Waves L1 (local platform cluster on real GitOps), LD (progressive-delivery architecture: canary + monitoring-gated, test-before-traffic, gradual shifting — structure now, controller later), L2 (app envs + promotion mechanics), L3 (promotion proven + drills). Orchestrator created the private GitHub repo `github.com/PatrickKoss/platform-infra` via `gh` and pushed the initial commit (b8929c4) after a secrets sweep (SOPS-encrypted placeholders only, `.local-keys/` gitignored, no plaintext credentials). Waves L1 + LD launched (2 ∥ codex agents; L1 owns the cluster + platform-infra git state, LD owns only `docs/progressive-delivery.md`, no git ops).
- 2026-08-09: **Wave I5a done** (1 codex agent, leitbild repo; run out of order because I4 is blocked on user credentials — I5a only needs the pushed `baukit-v0.3.0` tag). leitbild migrated 0.2.0 → 0.3.0: git deps + vendored TS packages bumped; `@baukit/auth-web` replaces `web/src/auth.ts`; `baukit-jobs` replaces the hand-built F5/F6 outbox+worker kernel (migration `0006_adopt_baukit_jobs.sql` converts the legacy `ai_jobs` table in place); chart values on the 0.3.0 features (per-process egress/volumes/env, hook cleanup — validated against the exact tag via `git archive`, and the k3d smoke harness now renders the tag instead of the mutable checkout). Net adoption delta **−271 LOC** (805 added / 1076 removed). **Deviation (justified)**: `SqliteRecordStore` NOT adopted — the tagged `@baukit/data-contracts-expo-sqlite` is uninstallable via its documented git path (`workspace:*` dev dep → `EUNSUPPORTEDPROTOCOL`) and its `baukit_records` table has no migration seam from the product's `read_cache`; product-local Expo adapter retained, defect logged. Gate green (orchestrator re-ran the full leitbild CI mirror independently, exit 0): fmt/clippy `-D warnings`, tests `--include-ignored` (Docker), OpenAPI drift, web (build/lint/test/format) + mobile (tsc/lint/test), observability harness "Unresolved observability metric names: none" — the `db_pool_acquire_duration_seconds_bucket` gap is dead on both api and worker. Friction list (full detail in leitbild `docs/baukit-v0.3.0-migration.md`): HIGH TS-package git-path install broken; MEDIUM `baukit-jobs`-vs-legacy-table schema migration needed by hand, `WorkerRunner` completes jobs outside the handler transaction (product writes idempotent-but-not-atomic), no clock seam in `WorkerRunner` (retry tests run live with shortened intervals); LOW async logout call-site change, no chart values JSON schema at the tag, `time` crate exact-pin conflict (`cargo update -p time --precise 0.3.47`). Focus J candidates filed from these: installable TS packages tested from an external product, `SqliteRecordStore` legacy-table migration seam, outbox→baukit-jobs upgrade cookbook, handler-controlled transactional completion, `WorkerRunner` clock seam, chart values JSON schema. leitbild tree left uncommitted.
- 2026-08-09: **Wave I3 done** (2 ∥ codex agents, disjoint areas, zero collisions; repo `/home/patrick/projects/platform-infra` git-initialized locally — GitHub remote NOT created, user action). **I3a** `terraform/`: OpenTofu modules (node, dns, object-storage); testing + production = Hetzner **cx43** (8 vCPU/16 GiB, ~€16.49/node/mo incl. protected IPv4; I2 footprint ruled out 8 GiB); PostHog node gated off until I8; Primary IPs as separate delete-protected resources (stable ingress identity across rebuilds); cloud-init template: SSH-key-only, no root, unattended security upgrades (reboots to kured), fail2ban, k3s v1.36.3+k3s1 with `disable: [traefik]`; firewall 80/443 public + SSH allowlist var, 6443 closed (admin via SSH tunnel); buckets via minio provider 3.40.1 (documented for Hetzner Object Storage); remote state = Hetzner S3 backend + client-side AES-GCM encryption (state bucket bootstrap out-of-band, ADR in terraform/README). Verified: `tofu fmt`/`validate`/`tflint`/`tofu test` (offline mocked plan: exactly 8 creates) — orchestrator re-ran green; live `tofu plan` needs the real hcloud token (64-char check; agent correctly refused to fabricate). **I3b**: cluster entrypoints `clusters/{testing,production}/` (platform.yaml/apps.yaml Kustomizations, SOPS decryption via `flux-system/sops-age`, flux-system placeholder for bootstrap output; local = README → baukit platform-up.sh); `platform/{testing,production}/` overlays fulfilling every base overlay contract (ACME staging on testing / prod on production, AlertmanagerConfig with encrypted webhook, kured window Sun 03:00–05:00 UTC, kyverno Audit, hcloud-volumes default StorageClass, monthly restore-test on); baukit GitRepository **forward-pinned to `baukit-v0.3.1`** (platform bases are working-tree-only; reconciles only after the next baukit release — I4 prerequisite); SOPS+age wired with real throwaway keys (gitignored `.local-keys/`, public keys in `.sops.yaml` marked TODO-replace); `renovate.json` consuming a shared preset added to baukit (`renovate-preset.json` + baukit `renovate.json`, `local>patrickkoss/baukit:renovate-preset`); 6 runbook skeletons; repo `validate.sh` (kustomize+kubeconform over 14 dirs, recursive `flux build` with local-source substitution → 103 resources/env, SOPS-policy + plaintext-secret grep gates). **Orchestrator fix**: I3b had composed the full hcloud base; per the I0 pin-table decision CCM stays uncomposed (future managed-LB only) — added `$patch: delete` for the CCM HelmRelease in both env overlays (CSI kept), avoiding the k3s embedded-cloud-controller coupling I3b flagged; re-verified builds (CCM absent, CSI present). Gate green (orchestrator-verified): platform-infra validate.sh, SOPS decrypt round-trip both envs, Renovate configs valid JSON + validator-passed, baukit `make ci` untouched by the two root JSON files. **I4 is blocked on user actions** (see wave I3/I4 user-action bullets): private GitHub repo + push, baukit-v0.3.1 release, real domain + ACME e-mail, real age keys, Hetzner/S3/deploy-key credentials, state bucket, Renovate app install.
- 2026-08-09: **Wave I2 done** (1 codex agent). Local flavor: `deploy/platform/overlays/local/` + idempotent `platform-up.sh up|down|status` on k3d, Flux-native path preserved (real `flux install` v2.9.4 + GitRepository reconciling the local checkout via a read-only smart-HTTP `git http-backend` snapshot on `host.k3d.internal:9418` — dumb HTTP fails go-git, `git://` rejected by the source schema); all secrets runtime-generated to a gitignored state dir. MinIO deployed locally → barman-plugin backup **proven** (backup completed, `data.tar.gz` + WALs in bucket, restore-test recovered a throwaway cluster in 56 s, freshness metric pushed). Integration proof all-PASS: fixture (`--backend --web --worker --auth oidc`) built + deployed with the **unmodified** baukit-app chart against platform Keycloak (realm-as-code) + CNPG role/DB; headless-PKCE + authenticated CRUD green; worker job end-to-end (`item.created` → `succeeded`); observability verifier "unresolved metric names: none"; identity labels `product/service/environment` correct on live scrapes; 12-panel dashboard queries fixture series; burn-rate + worker alerts loaded healthy; second `up` converges, `down` removes cluster. Fixes in the right layer — **CI-matrix-relevant template fixes**: generated backends had no Dockerfile (added multi-target api/migrate/worker `Dockerfile.jinja`), migration image lost the compile-time crate dir, PG pool metrics not registered (enabled `baukit-ops/sqlx-postgres`); chart fixes: ServiceMonitor identity relabeling + `honorLabels` for worker job label; base fix: Loki/Tempo dependsOn kube-prometheus-stack (ServiceMonitor CRD race). Both fixture matrices (oidc+worker and CI backend+web+mobile) re-verified incl. Docker-gated tests. Footprint recorded in `deploy/platform/local-footprint.md`: pods 3.3 GiB/63m, node 6.3 GiB/123m idle; top: Prometheus 694 Mi, Keycloak 645 Mi, Grafana 353 Mi → **testing node ≥4 vCPU/16 GiB (avoid 8 GiB)**. Caveats: WSL2 cgroup v1 forces k3s v1.34.8-k3s1 locally (real nodes stay on the I0 v1.36 channel); `kubectl top` = working set, not RSS. Gate green (orchestrator-verified independently): validate.sh 10 bases/12 charts/2 overlays zero errors, metric-names lint, 0.3.0 coherence (4 charts), full `make ci`.
- 2026-08-09: **Wave I1 done** (3 ∥ codex agents, disjoint dirs, zero collisions). 10 platform bases under `deploy/platform/` (cluster-baseline, cert-manager, traefik, kyverno+policy-pack, kured, hcloud, cnpg+barman-plugin, postgres-cluster component, keycloak-operator vendored 26.7.1, observability-stack) all on I0 pins; `validate.sh` (auto-discovery, offline helm cache, pin enforcement) wired into `make ci`; orchestrator added `overlays/everything` fixture (83 resources, composes clean) + overlay validation in validate.sh. Gate green: validate.sh 10 bases/12 charts/1 overlay zero errors, metric-names lint, 0.3.0 coherence, full `make ci`. Notables: k3s embeds kube-proxy → its scrape target disabled alongside scheduler/controller-manager; barman plugin needs cert-manager (dependsOn ordering documented); ObjectStore ships deliberately unusable `s3://__OVERLAY_REQUIRED__/` marker; kyverno policies default Audit; restore-test freshness via Pushgateway (`baukit_restore_test_last_success_timestamp_seconds`, live push smoke-tested); Traefik render-verified hostPorts 80/443 only, no hostNetwork, ops listeners internal. Overlay contracts consolidated in per-base READMEs.
- 2026-08-09: **Wave I0 done** (1 codex agent, gpt-5.6-sol high). All perishable claims verified live; pin list recorded under I1; analysis §17 entries 21–29 appended. Material corrections: OCIRepository is GA (git-source kept for review-path reasons, not maturity); Loki/Tempo charts moved to `grafana-community` repo (Alloy stayed); CNPG in-tree barman deprecated (removal 1.31) → build on `plugin-barman-cloud` v0.14.0; `KeycloakRealmImport` is create-only (realm updates need versioned admin-API migration jobs); no Hetzner LB (LB11 €7.49/mo can't fix single-node SPOF) → Traefik on host ports 80/443; Keycloak via official namespace-scoped operator 26.7.1 (Bitnami ruled out, Broadcom registry). Open risks logged in §17.21–29: host-port ingress has no managed failover (Primary IP must survive rebuilds), Renovate hosted 30-min job cap, stack-vs-node-budget must be measured at I2.
- 2026-08-09: Phases 0–2 complete (Focus A–H); detailed task lists + wave logs moved verbatim to [implementation-tasks-archive-phase0-2.md](./implementation-tasks-archive-phase0-2.md). Focus I (Phase 3: GitOps production platform) planned; architecture decisions recorded as analysis §17 entry 20 (two-layer base/overlay split between baukit `deploy/platform/` and private `platform-infra`, three clusters local/testing/production, Flux+SOPS, app-repo-owned gitops dirs with PR promotion, Kargo/image-automation deferred, hosted Renovate, terraform-managed DNS, GHCR). Nothing implemented yet.
