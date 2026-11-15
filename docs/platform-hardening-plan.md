# Platform hardening plan

Owner: Claude (orchestrator). Implementers: subagents per the current orchestrator setup, at most 8 in
parallel, with disjoint directory ownership. The orchestrator re-runs every acceptance command, reads the
diff and looks for new suppressions or skipped checks before ticking a box.

Status markers: `[ ]` open, `[~]` in progress, `[x]` done and verified, `[-]` dropped (reason in the log).

Scope, in this order:

1. baukit `deploy/` (platform bases, `baukit-app` chart, observability packs, smoke harness) plus the
   templates that feed it (`templates/backend/deploy/`).
2. A baukit release that carries the result.
3. `platform-infra`, which re-pins to that release and adopts the new bases (phase PI).

Evidence below was collected on 2026-11-14 against baukit `16cc479e` and platform-infra `ecdb4c7`.
Paths without a repo prefix are relative to the baukit root.

## Principles

1. **One policy source, two enforcement points.** The Kyverno policies that admit or reject objects in the
   cluster also run offline in the gate, against every rendered base, overlay and chart profile. If the
   gate is green, admission will not reject the same manifests.
2. **Every guardrail ships with a test that proves it bites.** A policy gets a failing and a passing
   fixture. A NetworkPolicy gets a probe that must be denied. A Falco rule gets a trigger that must
   produce an event. An alert gets a `promtool` test where it fires and one where it stays quiet.
3. **Every component ships its own telemetry.** A base that installs a controller also enables its
   ServiceMonitor or PodMonitor, adds a PrometheusRule file with tests, a dashboard and a runbook per
   alert. A base without metrics is incomplete.
4. **Default deny, explicit allow.** Every namespace a base owns gets a Pod Security Admission level, a
   default-deny NetworkPolicy and named allow rules. Privileged namespaces are an explicit, short list.
5. **No silent failure modes.** A missing alert receiver, a dead rule selector or a skipped schema has to
   fail loudly. "Validated with `-ignore-missing-schemas`" counts as not validated.
6. **Enforce, don't audit forever.** A policy lands in `Enforce` once the e2e cluster shows zero
   violations. Remaining violations get a `PolicyException` with a reason, and the exception count may
   only go down (same rule as `baukit-check.toml` suppression budgets).
7. **Pinned and reproducible tools.** Every validator runs at a pinned version with a checksum, the same
   way locally and in CI. No "whatever is on PATH".
8. **Fits one node.** The platform has to run on the local WSL2 k3d node and a small Hetzner node. Every
   new component records its measured footprint in `deploy/platform/local-footprint.md`.
9. No product is live (see the compatibility stance). Breaking changes to bases, chart values and
   namespaces land directly, without transition switches.

## Decisions

These are defaults the plan assumes. Each one can be overruled before its phase starts.

| ID | Decision | Default | Why |
|---|---|---|---|
| D1 | CNI | Keep k3s' built-in kube-router NetworkPolicy controller. No Cilium. | Cilium on WSL2 k3d with cgroup v1 is a big, risky change. Standard NetworkPolicy covers namespace and pod isolation. Internet egress uses `ipBlock 0.0.0.0/0` minus RFC 1918 ranges on port 443. FQDN egress can be revisited when real nodes exist. |
| D2 | Falco driver | `modern_ebpf`. | Verified on this host: `/sys/kernel/btf/vmlinux` exists and `CONFIG_DEBUG_INFO_BTF=y` on 5.15.167.4-microsoft-standard-WSL2. No kernel module and no driver loader. |
| D3 | Falco output path | Falco native Prometheus metrics, plus falcosidekick with Loki and Alertmanager outputs. Web UI off. | Events land next to the other logs in Grafana. Critical events page through the same Alertmanager routing. |
| D4 | Kyverno policy form | Write new policies as the CEL policy types the pinned Kyverno recommends (`ValidatingPolicy`, `GeneratingPolicy`). Migrate the three existing `ClusterPolicy` objects. | Kyverno chart 3.10.0 (v1.20.0) already ships its Pod Security policies as `ValidatingPolicy` (`vpolExclude` values). Check the deprecation status of `ClusterPolicy` in the v1.20 release notes before K1 starts. |
| D5 | Mutation | No mutating policies, apart from generating default objects (NetworkPolicy, LimitRange) in tenant namespaces. | Explicit over implicit. A mutated `seccompProfile` hides a chart that is wrong. |
| D6 | Image signature verification | Parked until product images live in a registry. Today they are built locally and loaded with `k3d image import`. | `verifyImages` needs a registry and signatures. The policy and its test land in Audit mode so they are ready. |
| D7 | Traefik exposure | Switch from DaemonSet with hostPorts to a Deployment behind k3s ServiceLB (`type: LoadBalancer`). | It fixes the rollout hang (F-01). The `traefik` namespace can then run `restricted`, because the svclb pod holds the hostPort in `kube-system`. |
| D8 | PostHog | Out of scope for replacement. Fence it with NetworkPolicy, PSA `baseline` and a recorded exception list. | The self-host chart is sunset (F-30). Replacing it is a product decision, not hardening work. |
| D9 | Platform e2e cost | A separate `platform-e2e` gate lane, selected by `--affected` for `deploy/**`. It holds `k3d-full.lock`, waits while both machine-wide heavy-run slots (`/tmp/claude-heavy-slots/slot-{1,2}`) are taken, and is not part of the 30-minute full-gate budget until it has been measured. | A disposable k3d cluster with the full stack takes minutes to converge. It must not starve product gates. |

## Findings catalog

Only verified findings are listed. F = defect or gap, with evidence. Fix IDs point to the tasks below.

### Defects that exist today

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| F-01 | Traefik rollouts hang. A DaemonSet with `maxSurge: 1`, `maxUnavailable: 0` and hostPorts 80/443 cannot schedule the surge pod, because the old pod still holds the ports. | `deploy/platform/traefik/helm-release.yaml:26-31,51,59` | R1 |
| F-02 | The Postgres restore-test alert never loads. Its PrometheusRule lives in namespace `postgres`, but `ruleNamespaceSelector` only admits `observability`. | `deploy/platform/postgres-cluster/kustomization.yaml:3`, `deploy/platform/observability-stack/kube-prometheus-stack.yaml:225-231` | O1 |
| F-03 | A single-instance CNPG cluster keeps the default primary PDB, which blocks every node drain. Weekly kured reboots time out. | `deploy/platform/postgres-cluster/cluster.yaml:6`, no `enablePDB` | R2 |
| F-04 | Alertmanager sends every route, Watchdog included, to the `null` receiver. An overlay that forgets the patch gets no alerts and no error. There are no `inhibit_rules`. | `deploy/platform/observability-stack/kube-prometheus-stack.yaml:52-71` | O5 |
| F-05 | The app chart's NetworkPolicy admits ingress from Traefik in `kube-system`, while the Traefik base runs in namespace `traefik`. With the base, ingress to the API is denied unless the overlay patches it. | `deploy/chart/baukit-app/values.yaml` (`networkPolicy.ingressController`), `deploy/platform/traefik/namespace.yaml:4` | C1 |
| F-06 | The app chart defaults every image to `tag: latest`, the template writes `tag: latest` into every product, and the Kyverno base flags exactly that. | `deploy/chart/baukit-app/values.yaml` (`api.image.tag` etc.), `_helpers.tpl` (`baukit-app.image` defaults to `latest`), `templates/backend/deploy/values.yaml:17,26,34` | C2 |
| F-07 | The restore-test Role can patch or delete any CNPG `Cluster` in its namespace, the production cluster included. | `deploy/platform/postgres-cluster/restore-test.yaml:8-19` | R2 |
| F-08 | `hcloud/namespace.yaml` declares `kube-system` as a managed object, so prune tries to delete it. | `deploy/platform/hcloud/namespace.yaml` | R8 |
| F-09 | The local git snapshot server binds `0.0.0.0` without auth and tars the whole checkout, gitignored files included. Flux applies the result as cluster-admin. | `deploy/platform/platform-up.sh:102-109,126`, `platform-lifecycle.sh:196-203,225` | S6 |
| F-10 | The k3d API and ports 80/443 bind `0.0.0.0`. `platform status` prints the Grafana and RustFS passwords. | `platform-lifecycle.sh:250-251`, `platform-up.sh:197-198` | S6 |
| F-11 | Prometheus has a time retention but no `retentionSize`. Loki and Tempo have no size cap. A full PVC crashes them. | `kube-prometheus-stack.yaml:209,224`, `loki.yaml:47`, `tempo.yaml:40` | R6 |
| F-12 | redis-ha always makes `redis-0` master on start and rebuilds the Sentinel config from scratch. Restarting `redis-0` after a failover brings back an empty master. | `deploy/platform/redis-ha/statefulset.yaml:42-49,98-106` | R7 |
| F-13 | platform-infra pins `tag: baukit-v0.5.0`, but baukit only publishes `v*` tags. It still composes `deploy/platform/keycloak`, which BK21 removed. Its own `validate.sh` is red. | platform-infra `platform/{local,testing,production}/source.yaml:11`, `platform/local/reconciliation.yaml:483` | PI1 |
| F-14 | `platform-validate` runs only in the local gate. `ci.yml` has no job that calls it. | `.github/workflows/ci.yml` (no `--step platform-validate`), `scripts/ci-gate.sh:107` | V6 |

### Security gaps

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| F-20 | No namespace carries Pod Security Admission labels. k3s defaults to privileged everywhere. | `git grep pod-security deploy/` finds nothing outside the Kyverno README | N1 |
| F-21 | Kyverno has three `Audit` policies and nothing else: no Pod Security, no registry allowlist, no RBAC or Service hardening. The local cluster does not install Kyverno at all. | `deploy/platform/kyverno/policies.yaml`, platform-infra `docs/local-cluster.md:361` | K1-K6 |
| F-22 | Only `posthog`, `platform-redis` and the app chart ship NetworkPolicies. `postgres`, `observability`, `cert-manager`, `cnpg-system`, `traefik`, `flagger-system`, `kured` and `platform-system` are open to every pod. Any pod can push a fake restore-success timestamp to the unauthenticated Pushgateway, or read Loki. | `git grep -l NetworkPolicy deploy/platform` | N2-N4 |
| F-23 | No runtime detection. Falco is absent. | n/a | FA1-FA5 |
| F-24 | All ten HelmRepositories are plain HTTPS indexes. No OCI source and no `verify`. Neither Flux Git source verifies signatures. | `deploy/platform/*/helm-repository.yaml`, platform-infra `gotk-sync.yaml`, `source.yaml` | SC1, SC4 |
| F-25 | Chart images are tag-pinned, not digest-pinned (traefik, cert-manager, cnpg, kured, flagger, kube-prometheus-stack, loki, tempo, alloy, hcloud, the CNPG PostgreSQL image, barman plugin, pushgateway, `rancher/k3s` in the restore test, `alpine` in the snapshot image). | audit list, e.g. `cnpg/helmrelease.yaml:33`, `postgres-cluster/pushgateway.yaml:25` | SC2 |
| F-26 | Loki runs with `auth_enabled: false` and no NetworkPolicy, so any pod can query or flood logs. Traefik access logs include query strings, which can carry OAuth codes. | `observability-stack/loki.yaml:31`, `traefik/helm-release.yaml:86` | N3, S3 |
| F-27 | Traefik has no TLS options (min version, SNI strict), no HTTP to HTTPS redirect and no security-headers middleware. The rate limit keys on remote IP, which is the k3d LB IP locally. | `traefik/helm-release.yaml` | S3 |
| F-28 | Flagger's security context is not `restricted`-compliant (no seccomp, no drop ALL, no `allowPrivilegeEscalation: false`). | flagger chart values 55-60, `flagger/helm-release.yaml` | N1 |
| F-29 | k3s runs without secrets encryption at rest, without an audit policy and without a PodSecurity admission config. The Terraform cloud-init installs k3s with `curl | sh` and no checksum. | platform-infra `terraform/modules/node/cloud-init.yaml.tftpl:70-77,94`, `platform-lifecycle.sh` k3d args | S5 |
| F-30 | PostHog runs the sunset `charts-clickhouse` chart, `postgres:14.1` (2021 patch release), ClickHouse 22.8 (EOL), `secureCookies: false`, and an any-to-any internal policy over unauthenticated redis, kafka and zookeeper. | `posthog/helm-repository.yaml:8`, `dependencies.yaml:98,205`, `network-policy.yaml:16-20` | N4 (fence), D8 |
| F-31 | Product repos reconcile as cluster-admin with the cluster age key. No `serviceAccountName`, no `targetNamespace`, no Flux multi-tenancy lockdown flags. | platform-infra `apps/local/leitbild-*.yaml:16-40`, `clusters/local/flux-system/kustomization.yaml` | S4, PI4 |

### Reliability gaps

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| F-40 | No HelmRelease sets `driftDetection`. None sets `test.enable`. `cleanupOnFail` is missing on kube-prometheus-stack, loki, tempo, alloy and baukit-observability. Traefik and flagger keep CRDs in `crds/`, but `upgrade.crds` defaults to `Skip`, so CRDs never upgrade. | `deploy/platform/*/helm-release.yaml` | R3 |
| F-41 | No PodDisruptionBudgets in any base. `platform-critical` is defined but never used. Only cert-manager gets a priority class, and only in the local overlay. | `cluster-baseline/priority-classes.yaml`, `overlays/local/components/cert-manager/kustomization.yaml:15` | R4 |
| F-42 | cert-manager, the CNPG operator, CNPG instances, the barman plugin, Kyverno and Flux are not scraped. No PrometheusRule exists for any platform component. Kube and node health come only from untuned kube-prometheus-stack defaults. | `cert-manager/helm-release.yaml:63-66`, `cnpg/helmrelease.yaml:34-37`, `platform-up.sh:222-224` | O2-O4 |
| F-43 | Backup and WAL-archive failures go unseen until the monthly restore test, and that test's alert is dead (F-02). The ScheduledBackup has no `immediate: true`. Local backups sit on the same node as the database. | `postgres-cluster/scheduledbackup.yaml`, platform-infra `platform/local/reconciliation.yaml:421-422` | R5, O3 |
| F-44 | kured reboots without checking Prometheus for firing alerts (`prometheusUrl` empty) and notifies nobody. | `kured/helm-release.yaml:37` | R4 |
| F-45 | All 16 app alerts point at `https://runbooks.example.invalid/...`. No runbook content exists. The template's `ApiHighErrorRatio` uses `severity: page`, outside the critical/warning/info scheme. | `deploy/observability/alerts/baukit.rules.yml`, `templates/backend/deploy/observability/rules/api.rules.yml` | O6 |
| F-46 | The app chart has no `values.schema.json`, no dedicated ServiceAccount, no `startupProbe`, no `topologySpreadConstraints`, no `preStop` delay, and PDBs are off even when `replicas > 1`. | `deploy/chart/baukit-app/` | C3 |
| F-47 | The smoke harness never proves that the app chart passes `restricted`. It does not label the namespace. | `deploy/smoke/k3d-smoke.sh:103` | C4 |

### Validation gaps

| ID | Finding | Evidence | Fix |
|---|---|---|---|
| F-50 | kubeconform skips 21 CRD kinds and runs with `-ignore-missing-schemas`, so HelmRelease, Kustomization, ClusterPolicy, ServiceMonitor, PrometheusRule and CNPG objects are never schema-checked. | `deploy/platform/validate.sh:40` | V2 |
| F-51 | Validators run at whatever version is on PATH. On this host, `kyverno`, `chainsaw`, `promtool`, `trivy` and `yamllint` are missing. `check-rules.sh` uses an unpinned local `promtool` before its pinned Docker fallback. | `deploy/observability/lint/check-rules.sh:5` | V1 |
| F-52 | Policies are never tested (`kyverno test`) and never applied to rendered output (`kyverno apply`). | n/a | V3 |
| F-53 | Rule and dashboard files are registered by name in three places (PrometheusRule template, ConfigMaps, `check-rules.sh`). A new file is silently ignored. | `deploy/observability/templates/prometheusrule.yaml:2-3,19`, `configmaps.yaml:17-18,38,54` | O2 |
| F-54 | No cluster-level test. No check that a NetworkPolicy actually denies, that admission actually rejects, or that an alert reaches a receiver. | n/a | E1-E3 |
| F-55 | The metric-name lint rejects every third-party metric (`kube_*`, `certmanager_*`, `falcosecurity_*`, ...) and LogQL parsers other than `json`. Platform rules have no linted home. | `deploy/observability/lint/check-metric-names.py:17-71` | O2 |
| F-56 | Chart tests are Python `unittest` over `helm template` text. They are fine, but they cover only PDB, MCP, platform, redis and web. NetworkPolicy, security context and the env contract are untested. | `scripts/test_chart_*.py` | C5 |

## Target architecture

```text
deploy/
  tools.toml                     pinned validator versions + sha256 (V1)
  platform/
    validate.sh                  existing entrypoint, now: schemas, kyverno apply, conventions (V2-V5)
    lint/
      check_conventions.py       cross-resource rules (namespace has PSA + default-deny, etc.)
      test_check_conventions.py
      ci-policies/               Kyverno policies that only run offline (HelmRelease conventions)
    schemas/                     generator config for CRD JSON schemas (output in .helm-cache)
    components/
      network-policy/            reusable: dns-egress, apiserver-egress, prometheus-scrape, internet-https
      tenant/                    tenant namespace: PSA restricted, default-deny, quota, LimitRange, Flux SA
    kyverno/                     controller + policy-reporter
    kyverno-policies/            new base: baukit policy library + upstream PSS chart
      policies/<name>.yaml
      tests/<name>/{kyverno-test.yaml,resources.yaml}
    falco/                       new base (FA1)
      rules/baukit.rules.yaml
    <every base>/network-policy.yaml   default-deny + named allows (N2)
    e2e/
      run.sh                     disposable k3d cluster, Flux, local snapshot, chainsaw
      overlay/                   what the e2e cluster composes
      tests/<feature>/chainsaw-test.yaml
  observability/                 app pack (unchanged scope)
  observability-platform/        new chart: platform alerts, recording rules, dashboards, tests
    metrics.txt                  exact third-party metric allowlist
    alerts/ recording-rules/ dashboards/ tests/ loki-rules/
docs/runbooks/platform/<slug>.md  one per alert
```

## Phase V: validation toolchain

Goal: every later phase lands with offline checks that run in seconds, locally and in CI, at pinned
versions. This phase comes first so every later task has a place to put its tests.

- [x] **V1 Pinned tools.** Add `deploy/tools.toml` listing kubeconform, kustomize, helm, kyverno CLI,
  chainsaw, promtool, pint, trivy, k3d, flux and yamllint with exact versions and per-platform sha256.
  Add `scripts/install-deploy-tools.sh`, which installs into gitignored `.tools/bin` and verifies checksums.
  `validate.sh`, `check-rules.sh` and the e2e runner put `.tools/bin` first on PATH and fail with exit 2
  if a tool is missing, instead of printing install hints. Drop the unpinned-`promtool` branch in
  `check-rules.sh`. Falco rule validation uses a digest-pinned `falcosecurity/falco` image.
  Acceptance: `scripts/test_install_deploy_tools.py` covers checksum mismatch, unknown platform and
  idempotent re-run. `scripts/check-version-coherence.py` or a new test fails if `tools.toml` and the
  CI workflow disagree.
- [x] **V2 Real schemas for every kind.** Add `deploy/platform/schemas/generate.py`. It extracts CRDs
  from the cached chart tarballs, the vendored manifests (barman plugin) and `flux install --export` at
  the pinned Flux version, converts them to kubeconform JSON schemas in `.helm-cache/crd-schemas/`, and
  is deterministic. `validate.sh` passes `-schema-location default` plus the local template, deletes
  `kubeconform_skip_kinds` and drops `-ignore-missing-schemas`. Helm-rendered output (`helm template`)
  is piped through kubeconform as well, not just linted.
  Acceptance: a deliberately misspelled field in a HelmRelease, a ServiceMonitor and a CNPG Cluster each
  fails validation (unit test with fixtures under `deploy/platform/lint/fixtures/`).
- [ ] **V3 Policies run offline.** `validate.sh` gains a stage that runs `kyverno test` over
  `deploy/platform/kyverno-policies/tests/` and then `kyverno apply` with every in-cluster policy plus
  `lint/ci-policies/` against: each rendered base, each overlay, each HelmRelease's rendered chart
  output, and the `baukit-app` chart rendered with `tests/all-options-values.yaml` and the generated
  fixture's `deploy/values.yaml`. Any `fail` result without a matching `PolicyException` fails the
  gate. Exceptions are read from the same files the cluster gets.
  Acceptance: a rendered pod without limits fails the stage. The stage prints a per-policy pass/fail
  table to `.ci-gate/`.
- [x] **V4 Convention checker.** `deploy/platform/lint/check_conventions.py` (stdlib only, unittest
  tests, same style as `scripts/`). It covers cross-resource rules that one-object policies can't:
  - every Namespace a base owns has PSA `enforce` and `audit`/`warn` labels; `privileged` only for the
    allowlist (N1);
  - every namespace that runs pods has a default-deny NetworkPolicy covering Ingress and Egress (N2);
  - every HelmRelease whose chart exposes a metrics toggle has it enabled, or the base has a PodMonitor;
  - every alert in every rule pack has `severity` in {critical, warning, info}, `summary`,
    `description`, a `runbook_url` whose slug has a file in `docs/runbooks/`, and at least one promtool
    test case that fires it;
  - every PrometheusRule and dashboard ConfigMap lands in a namespace the Prometheus and Grafana
    selectors admit (prevents a repeat of F-02);
  - the PolicyException count does not exceed the budget in `deploy/platform/lint/budget.toml`, which
    may only go down.
- [x] **V5 HelmRelease conventions as offline policies** in `lint/ci-policies/`: exact chart version;
  `install.remediation.retries`, `upgrade.remediation.retries`, `upgrade.cleanupOnFail`; explicit
  `timeout`; `driftDetection.mode: enabled`; `install.crds` and `upgrade.crds: CreateReplace` when the
  chart ships CRDs; HelmRepository `interval >= 1h`; Kustomization `prune: true` with `wait: true` and no
  dead `healthChecks` list (F-M3 in platform-infra). These run on Flux objects only, so they never load
  into the cluster.
- [ ] **V6 CI parity.** Add a `platform` job to `.github/workflows/ci.yml` that installs tools through
  `scripts/install-deploy-tools.sh` (cached by `tools.toml` hash) and runs `--step platform-validate`
  and `--step observability-rules`. Wire `platform-e2e` (phase E) as a separate job selected by path.
  Update the CI table in `CLAUDE.md`. Acceptance: `actionlint` clean, `scripts/test_ci_gate.py` covers the
  new steps.

## Phase E: cluster e2e harness

Goal: one command builds a disposable cluster with the composed platform and proves the guardrails
behave. Later phases add test directories here.

- [~] **E1 Runner.** `deploy/platform/e2e/run.sh` creates the k3d cluster `baukit-platform-e2e` with the
  same k3s image and hardening flags as the lifecycle (S5), binds everything to `127.0.0.1`, installs Flux
  at the pinned version (including notification-controller), serves the working tree through the
  local-snapshot source (with S6's file filtering), applies `e2e/overlay/`, waits for every Kustomization
  and HelmRelease to be Ready, runs `chainsaw test deploy/platform/e2e/tests`, collects
  `kubectl get events,polr,cpolr -A`, Flux status and pod logs into `.ci-gate/platform-e2e/`, and deletes
  the cluster on exit (trap, also on SIGINT). Every run takes `flock` on `k3d-full.lock` and waits while both
  heavy-run slots are taken before creating the cluster. It does not take the android or browser locks,
  which belong to the slot holders.
- [~] **E2 Overlay.** `deploy/platform/e2e/overlay/` composes every base except `hcloud` and `posthog`
  (posthog is checked separately in N4) plus a test tenant namespace built from `components/tenant`, a
  tiny echo app deployed with `baukit-app` (image built in the runner from a pinned `nginx-unprivileged`
  digest, so it needs no product), and a webhook sink (pinned image) that records Alertmanager
  notifications for assertions.
- [~] **E3 Gate lane.** Add `platform-e2e` to `scripts/ci-gate.sh` and `ci-gate.toml`, selected for
  `deploy/platform/**`, `deploy/chart/**` and `deploy/observability*/**`. Measure wall time over three
  runs and record it in the log. If the median exceeds 15 minutes, split the chainsaw suites into two
  lanes before adding it to the full gate (D9). Add a `make platform-e2e` target and a CLAUDE.md row.
- [~] **E4 Baseline tests**, present from day one so every later phase has a pattern to copy:
  `smoke/` asserts all Flux objects are Ready and no pod restarted more than once;
  `alert-pipeline/` creates a PrometheusRule with an always-firing `BaukitE2eProbe` alert and asserts the
  webhook sink receives it within 3 minutes (this proves rule selection, Prometheus, Alertmanager
  routing and receiver end to end).

## Phase C: app chart and templates

- [~] **C1 Ingress controller default.** Change `networkPolicy.ingressController.namespaceSelector` to
  `kubernetes.io/metadata.name: traefik` and the pod selector to the Traefik base's labels. Add a chart
  test. The e2e echo app must be reachable through Traefik with the chart's defaults (`e2e/tests/ingress/`).
- [x] **C2 No mutable tags.** Remove the `latest` default from `baukit-app.image`. Each image takes
  `tag` or `digest`, at least one is required, and `latest` fails the render with a clear message.
  `templates/backend/deploy/values.yaml` writes a placeholder the product's release pipeline replaces
  (for example `tag: "0.0.0-unset"` plus a comment), and the fixture gate renders the chart with the
  generated values to prove it. Regenerate the CLI snapshots and check the diff.
- [x] **C3 Chart hardening.**
  - `values.schema.json` covering every key, with `additionalProperties: false` at the top level.
  - A dedicated ServiceAccount per release with `automountServiceAccountToken: false`.
  - `startupProbe` on API and worker, so slow boots stop tripping liveness.
  - A soft `topologySpreadConstraints` default on `kubernetes.io/hostname`.
  - A `preStop` sleep (default 5 s, below the drain timeout) so endpoints drop before the drain starts.
  - PDB on by default with `maxUnavailable: 1` when `replicas > 1` or autoscaling is on.
  - `ttlSecondsAfterFinished` on migration and seed jobs.
  - An `emptyDir` `/tmp` mount, so `readOnlyRootFilesystem` keeps working when a library writes temp files.
  - Fix the stray `{{ with` (missing `-`) before `api.networkPolicy.additionalEgress` in
    `templates/networkpolicy.yaml`.
- [~] **C4 Smoke harness under `restricted`.** `deploy/smoke/k3d-smoke.sh` labels its namespace
  `pod-security.kubernetes.io/enforce=restricted` before installing, and installs the baukit Kyverno
  policy library in Enforce mode first. A chart change that breaks either fails the smoke.
- [x] **C5 Chart tests.** Extend `scripts/test_chart_*.py`: security context on every container,
  NetworkPolicy peers per process, env contract (prefix, overrides, secret refs), schema rejection of
  unknown keys and of `latest`. Every C task adds its own cases.

## Phase K: Kyverno policy library

- [x] **K1 Controller.** Bump the Kyverno base to chart 3.10.0 (Kyverno v1.20.0), after reading its
  upgrade notes.
  - Enable ServiceMonitors for all four controllers.
  - Set `features.policyExceptions.enabled: true` with `namespace: kyverno`, so exceptions can only
    live in one namespace that tenants can't write to.
  - Set the admission webhook timeout explicitly and add PDBs for multi-replica overlays.
  - Keep the chart's default exclusion of `kube-system` and `kyverno` from webhooks, and add
    `flux-system`, so a broken policy can never block the controllers that would repair it.
  - Add Policy Reporter (pinned chart, metrics only, UI off), so dashboards show current violations
    and not only admission events.
- [x] **K2 Migrate the existing three policies** (`require-container-resources`,
  `disallow-latest-image-tag`, `protect-operations-ports`) to the D4 policy type in a new base
  `deploy/platform/kyverno-policies/`, each with a `kyverno test` suite.
- [x] **K3 Pod Security.** Install the upstream `kyverno-policies` chart (pinned, `podSecurityStandard:
  restricted`, `validationFailureAction: Audit` cluster-wide). It reports `restricted` gaps in every
  namespace, platform namespaces included. PSA labels (N1) remain the enforcement backstop. Tenant
  namespaces get `Enforce` through `validationFailureActionByPolicy` plus a namespace selector on
  `baukit.dev/tenant=true`.
- [x] **K4 Baukit policy set.** Each one ships with pass and fail fixtures, a message that names the fix,
  and the rollout mode it lands in:

  | Policy | Applies to | Mode at landing |
  |---|---|---|
  | `restrict-image-registries` (allowlist from a params ConfigMap, overlay-owned) | all pods | Audit, Enforce on tenants |
  | `require-image-digest` | tenant namespaces labelled `baukit.dev/environment=production` | Audit (until D6) |
  | `require-probes` (readiness + liveness on Deployment/StatefulSet) | tenants | Enforce |
  | `require-baukit-labels` (`app.kubernetes.io/name`, `part-of`, `baukit.dev/process`) | tenants | Enforce |
  | `disallow-default-namespace` | all | Enforce |
  | `restrict-service-external-ips` (CVE-2020-8554) | all | Enforce |
  | `restrict-loadbalancer-and-nodeport` (only `kube-system` and `traefik`) | all | Enforce |
  | `unique-ingress-host-and-path` | all | Enforce |
  | `disallow-sa-token-automount` | tenants | Enforce |
  | `restrict-wildcard-rbac` (no `*` verbs/resources in Roles outside system namespaces) | all | Audit |
  | `restrict-cluster-admin-binding` | all | Enforce |
  | `require-psa-labels` (Namespace must carry an `enforce` level; `privileged` only from the allowlist) | Namespaces | Enforce |
  | `restrict-netpol-wildcards` (no `from: [{}]` or empty namespaceSelector in tenant policies) | tenants | Enforce |
  | `flux-tenant-service-account` (Kustomization/HelmRelease in a tenant namespace must set `serviceAccountName` and may not reference sources in other namespaces) | tenants | Enforce |
  | `verify-baukit-images` (cosign keyless, identity = the product's release workflow) | tenants | Audit (D6) |

- [x] **K5 Generated defaults for tenants.** A `GeneratingPolicy` (or `ClusterPolicy` generate rule if
  D4 says so) creates `default-deny` NetworkPolicy, a LimitRange and the DNS egress allow in every
  namespace labelled `baukit.dev/tenant=true`, with `synchronize: true`. `components/tenant` labels the
  namespace. An overlay that forgets the component still gets a default deny.
- [ ] **K6 Exceptions and enforcement.** Run the e2e cluster, collect PolicyReports, and for every
  remaining violation either fix the base values or add a `PolicyException` in
  `deploy/platform/kyverno-policies/exceptions/` with `baukit.dev/reason` naming the invariant (for
  example "node-exporter needs hostPID to read host metrics"). Record the count in `budget.toml`. Then
  switch each policy to its target mode. e2e test `admission/` applies one violating manifest per
  Enforce policy and asserts rejection, and one compliant manifest and asserts admission.

## Phase N: Pod Security Admission and NetworkPolicies

- [~] **N1 PSA labels on every base namespace.** Every Namespace gets `enforce`, `enforce-version`
  (pinned to the cluster minor), `audit: restricted` and `warn: restricted`. Target `enforce` levels:

  | Level | Namespaces | Work needed |
  |---|---|---|
  | restricted | cert-manager, cnpg-system, postgres, platform-redis, kyverno, flagger-system, traefik (after D7), observability (after the split below), tenants | flagger `securityContext` values (F-28); traefik moves to ServiceLB |
  | baseline | posthog | chart pods lack security contexts (D8) |
  | privileged | kube-system, observability-agents, kured, falco | listed in the `require-psa-labels` allowlist |

  Split node-exporter and Alloy into a new `observability-agents` namespace (they need hostPID,
  hostNetwork and hostPath), so `observability` itself can run `restricted`. Flux's own namespace gets
  labels through the lifecycle's `flux install` patch.
- [x] **N2 Reusable NetworkPolicy components** in `deploy/platform/components/network-policy/`:
  `default-deny`, `dns-egress` (CoreDNS pods plus the k3s Service-IP compatibility rule the chart
  already documents), `apiserver-egress` (k3s DNATs `10.43.0.1:443` to the node IP on 6443, so the
  rule takes an `ipBlock` for the node CIDR, set by a Kustomize replacement from one overlay value),
  `prometheus-scrape` (ingress from Prometheus pods on named metrics ports) and `internet-https`
  (`0.0.0.0/0` except RFC 1918, 100.64/10 and link-local, port 443). The e2e test proves the apiserver
  rule against k3d before any base relies on it.
- [x] **N3 Per-base policies.** Each base composes `default-deny` plus explicit allows. Peer and port
  pairs from the audit (verify each one in e2e; they are a starting point, not a spec):
  - **traefik:** ingress 8000/8443 from anywhere, 9100 from Prometheus. Egress to the apiserver, DNS,
    and backend ports in namespaces labelled `baukit.dev/ingress-backend=true`.
  - **cert-manager:** controller egress to the apiserver, DNS and `internet-https` (ACME, DNS-01
    provider), plus 80/443 to Traefik for the HTTP-01 self-check. Webhook ingress 10250 from the node
    CIDR. Prometheus ingress 9402.
  - **cnpg-system:** operator egress to the apiserver, instances on 8000 and 5432, the plugin on 9090.
    Webhook 9443 ingress from the node CIDR.
  - **postgres:** 5432 ingress only from namespaces labelled `baukit.dev/postgres-client=true` and from
    the operator; 8000 from the operator; 9187 and 9091 from Prometheus. Pushgateway 9091 also from the
    restore-test job's pods, and only from those. Egress to the apiserver, object storage (rustfs 9000
    locally, `internet-https` on Hetzner) and DNS.
  - **observability:** Grafana to Prometheus 9090, loki-gateway 80 and Tempo 3200. loki-gateway 80
    only from Alloy and Grafana. Tempo 4317/4318 from tenant namespaces. Alertmanager 9093 from
    Prometheus. Alertmanager egress through `internet-https`. Prometheus egress to all scrape ports,
    kubelet 10250 and node-exporter 9100 on the node CIDR, the apiserver and CoreDNS 9153.
    Prometheus 9090 ingress from Grafana, flagger and kured.
  - **kyverno, kured, flagger, platform-redis, falco:** controller-specific allows plus
    `prometheus-scrape`.
- [x] **N4 Fence PostHog** (D8): replace the any-to-any internal policy with explicit pairs, deny egress
  except DNS and `internet-https` for the plugin server, bump `postgres:14.1` to the newest 14.x digest.
  Record the remaining `baseline` gaps as exceptions.
- [ ] **N5 Probe matrix test.** `deploy/platform/e2e/tests/netpol/matrix.yaml` lists source, destination,
  port and expected verdict, for example `tenant -> postgres:5432 allow (with label)`,
  `tenant -> postgres:5432 deny (without label)`, `tenant -> loki-gateway:80 deny`,
  `tenant -> pushgateway:9091 deny`, `tenant -> kube-apiserver deny`,
  `cert-manager -> internet:443 allow`, `tenant -> 169.254.169.254 deny`. A chainsaw script runs
  short-lived probe pods (pinned busybox digest, `restricted`-compliant) and fails on any mismatch.
  Every new allow rule in N3 adds a matrix line.

## Phase O: platform observability

- [x] **O1 Rule and dashboard discovery.** Set `ruleNamespaceSelector`, `serviceMonitorNamespaceSelector`
  and `podMonitorNamespaceSelector` to match `app.kubernetes.io/part-of: baukit-platform` namespaces,
  and label every base namespace with it. Grafana's sidecar searches `ALL` with the `grafana_dashboard`
  label and uses `folderAnnotation`, so each component gets its own folder. This fixes F-02. The V4
  convention check fails if a rule lands outside the selector again.
- [x] **O2 Platform observability pack.** New chart `deploy/observability-platform/`
  (`baukit-platform-observability`, version-coherent with the rest). Its templates glob
  `alerts/*.yml`, `recording-rules/*.yml` and `dashboards/*.json` with `.Files.Glob`, one PrometheusRule
  per file and one dashboard ConfigMap per file, each toggled per component in values.
  Also:
  - Switch `deploy/observability/templates/` to the same glob, which fixes F-53.
  - Generalize `check-rules.sh` to take a pack directory and run `promtool check rules` on everything
    plus `promtool test rules` on `tests/*.test.yml`.
  - Run `pint` with a checked-in config on both packs.
  - Lint the platform pack with `check-metric-names.py --observability-root deploy/observability-platform
    --allowlist deploy/observability-platform/metrics.txt`.
  - Extend `baukit check observability` with per-pack allowlists, so third-party names never leak into
    the app pack's allowed set.
  - Extend the LogQL keyword support (`logfmt`, `line_format`, `label_format`, `pattern`, `regexp`,
    `unwrap`, label filters after a parser) in both the Python lint and `cli/src/check/observability/`,
    with unit tests in both.
  - Ship the pack through a HelmRelease in the observability-stack base.
- [ ] **O3 Exporters on.**
  - cert-manager: `prometheus.servicemonitor.enabled`.
  - CNPG: operator `monitoring.podMonitorEnabled` and Cluster `monitoring.enablePodMonitor`, plus a
    barman plugin PodMonitor.
  - Kyverno ServiceMonitors (K1) and Falco metrics (FA1).
  - Flux: a PodMonitor for the controllers, plus a kube-state-metrics `customResourceState` config that
    emits `gotk_resource_info` for Kustomization, HelmRelease, GitRepository, OCIRepository and
    HelmRepository, as the Flux monitoring docs describe.
  - flagger `podMonitor.enabled`, hcloud CCM/CSI monitors (Hetzner overlays only) and a redis_exporter
    sidecar in both redis bases.
  - Capture the real metric names from a running e2e cluster into `metrics.txt` instead of guessing them.
- [~] **O4 Alerts, each with a promtool test that fires and one that stays quiet, and a runbook.**
  Names follow `Baukit<Component><Condition>`. Every alert carries a `component` label for routing.
  - Flux: reconcile not Ready for 15m (`gotk_resource_info{ready="False"}`), suspended for 24h,
    source fetch failing.
  - cert-manager: certificate expires in under 21d (warning) and under 7d (critical), certificate not
    Ready for 10m, ACME request errors.
  - CNPG: instance not ready, WAL archiving failing (archiver failed count increasing), last archived
    WAL older than 15m, last successful backup older than 26h, connection saturation above 85%, PVC
    above 80%.
  - Traefik: 5xx ratio per service, TLS cert `not_after` under 7d, config reload failures.
  - Kyverno: admission controller down, webhook p99 latency above 1s, rising `fail` results per policy,
    PolicyReport violations above zero for Enforce policies (from Policy Reporter).
  - Falco: see FA3.
  - Observability itself: Loki discarded samples, Alloy write failures, Prometheus TSDB near its
    `retentionSize`, rule evaluation failures.
  - kured: reboot required for more than 48h, or blocked.
  - Tune the kube-prometheus-stack `defaultRules`: keep KubePod*, KubePersistentVolume*, Node*,
    Prometheus* and Alertmanager*; disable rules for components k3s does not expose; and override their
    `runbook_url` to the baukit runbooks where baukit adds one.
  - Delete `BaukitDeadMansSwitch`, which duplicates kube-prometheus-stack's `Watchdog`.
  - Move the template's `ApiHighErrorRatio` to `severity: critical`, with a runbook URL.
- [ ] **O5 Routing that fails loudly** (F-04).
  - Alertmanager reads its config from an overlay-provided Secret (`alertmanagerSpec.useExistingSecret`
    / `configSecret`). Without it, Alertmanager does not start, Watchdog stops, and the external dead
    man's switch fires.
  - The base ships an example config with routes by `severity` and `component`, `inhibit_rules`
    (critical inhibits warning for the same `alertname`/`namespace`, `InfoInhibitor`) and a Watchdog
    route to an external heartbeat receiver.
  - Flux notification-controller sends `Provider` type `alertmanager` events for Kustomization,
    HelmRelease and GitRepository errors.
  - The e2e `alert-pipeline/` test covers the example config.
- [x] **O6 Runbooks.** `docs/runbooks/platform/<slug>.md` and `docs/runbooks/app/<slug>.md`, one per
  alert, with the same headings: meaning, impact, first checks (exact `kubectl`/LogQL/PromQL), fix,
  escalation. `runbook_url` becomes `{{ .Values.runbookBaseUrl }}/<slug>` (overlay value). The V4 check
  enforces that every slug has a file.
- [x] **O7 Dashboards.** One per component in its own folder: Flux, cert-manager, CNPG (upstream
  dashboard, pinned and vendored), Traefik, Kyverno/Policy Reporter, Falco, platform overview (all
  `Baukit*` alerts by component, Ready state of every Flux object, PSA and policy violation counts).
  `baukit check observability` already checks datasource variables. Add a JSON schema check for
  dashboard structure and a test that every panel's PromQL passes the metric lint.

## Phase FA: Falco runtime security

- [x] **FA0 Spike (timeboxed, one agent).** On a disposable k3d cluster, install Falco with
  `modern_ebpf` and record in the log:
  - whether k3s' containerd socket (`/run/k3s/containerd/containerd.sock` inside the node container)
    gives container and Kubernetes metadata;
  - how much host noise the shared WSL2 kernel adds (Docker Desktop, other k3d clusters, product gate
    containers);
  - CPU and memory at idle;
  - the actual names of Falco's Prometheus metrics at the pinned version.

  Stop and report if metadata enrichment does not work. The fallback is filtering to the k3d node
  container's cgroup.
- [x] **FA1 Base** `deploy/platform/falco/`:
  - Namespace `falco` with PSA `privileged` (allowlisted) and default-deny NetworkPolicy plus
    `prometheus-scrape`. Egress from falcosidekick to loki-gateway and Alertmanager only.
  - HelmRelease for the `falcosecurity/falco` chart (9.2.2 / Falco 0.45.2 as of 2026-11-14; pin what
    is current when the task starts) with:
    - `driver.kind: modern_ebpf` and the k3s containerd socket;
    - `metrics.enabled` with a ServiceMonitor;
    - falcosidekick enabled with the Loki and Alertmanager outputs (minimum priority `warning` for
      Alertmanager), web UI off;
    - bounded resources and `platform-standard` priority.
  - Images by digest (SC2).
- [x] **FA2 Rules.** Keep the upstream default ruleset and add `rules/baukit.rules.yaml`, loaded as an
  extra rules file:
  - macros that silence the known k3d and Flux noise from FA0, each with a comment naming the source;
  - app rules keyed on the `baukit.dev/process` label:
    - shell or package manager spawned in a baukit app container;
    - process other than the image entrypoint in `api` or `worker`;
    - outbound connection from a `migration` job to anything but PostgreSQL;
    - read of `/var/run/secrets/kubernetes.io` in a pod that has automount off;
    - write below `/etc` or `/usr`;
  - platform rules: `kubectl exec` or attach into `postgres`, `kyverno` or `flux-system`, plus a
    terminal shell in any platform namespace.

  The static lane validates rule files with `falco --validate` in the pinned image.
- [x] **FA3 Alerts and dashboard** in the platform pack:
  - `BaukitFalcoDown` (a node without a ready Falco pod);
  - `BaukitFalcoEventsDropped` (kernel drop counter increasing);
  - `BaukitFalcoCriticalEvent` (any event at priority Critical or above in 5m, critical);
  - `BaukitFalcoWarningEventsHigh` (warning-level rate above a threshold, warning);
  - `BaukitFalcosidekickOutputErrors`.

  Add the falco dashboard: events by rule, priority and namespace, drop rate, and a Loki panel with the
  latest events. Add the metric names from FA0 to `metrics.txt`.
- [ ] **FA4 Detection tests** in `e2e/tests/falco/`:
  - exec a shell in the echo app and assert an event with the baukit rule name appears in Loki and
    `BaukitFalcoCriticalEvent` reaches the webhook sink;
  - read `/etc/shadow` in a test pod and assert the upstream rule fires;
  - a 10-minute soak on the idle e2e cluster produces zero Warning+ events, so noise regressions fail
    the lane.
- [ ] **FA5 Footprint and docs.** Measure Falco and falcosidekick at idle and record it in
  `local-footprint.md`. Write the base README (overlay contract, the noise-tuning procedure, how to add
  an exception macro with a reason) and the runbooks.
- [ ] **FA6 Kubernetes audit events (optional, after FA4).** Configure the k3s API server audit webhook
  to Falco's `k8saudit` plugin, through `--k3s-arg` in the lifecycle and the Terraform k3s config. It
  adds rules for secrets read by unexpected service accounts, exec into pods and ClusterRoleBinding
  changes. Only proceed if FA0 showed headroom.

## Phase R: reliability

- [x] **R1 Traefik** (F-01, D7): a Deployment with 2 replicas where the node count allows (1 locally),
  `type: LoadBalancer` through k3s ServiceLB, rolling update `maxUnavailable: 0, maxSurge: 1`, PDB,
  `platform-critical`, `install/upgrade.crds: CreateReplace`. The e2e `ingress/` test bumps a pod
  annotation and asserts the rollout completes with no failed requests from a curl loop.
- [x] **R2 CNPG** (F-03, F-07): `enablePDB: false` when `instances: 1` (documented in the README, with
  `nodeMaintenanceWindow` for multi-instance overlays). Scope the restore-test Role's delete and patch
  to `resourceNames: [postgres-cluster-restore-test]`. Give the restore test a Delete-policy
  StorageClass on Hetzner. Add an e2e test that drains the node with `kubectl drain --dry-run=server`
  and asserts no PDB blocks it.
- [x] **R3 HelmRelease standards** (F-40): apply the V5 conventions to every base. `driftDetection` lands
  as `mode: warn` first, then `enabled` after one clean e2e run with no drift events. Set explicit
  `timeout`s from measured install times. Add `test.enable` where the chart ships tests.
- [ ] **R4 Disruption and priority** (F-41, F-44):
  - `platform-critical` for Traefik, CoreDNS (through the k3s manifest overlay if feasible), the CNPG
    operator, the postgres instance, Kyverno admission and cert-manager.
  - `platform-standard` for everything else in the bases.
  - PDBs for every multi-replica platform workload.
  - kured gets `prometheusUrl` (skip reboot while non-info alerts fire), a notify URL from the
    overlay, and a reboot window.
- [~] **R5 Backups that are known to work** (F-43): `immediate: true` on the ScheduledBackup. The restore
  test checks a sentinel row, not just Ready. Document an off-node backup target for the local overlay
  (host bind mount or a second object store) in the base README. Add an e2e test that runs backup,
  deletes the cluster, restores and compares the sentinel.
- [x] **R6 Storage limits** (F-11): `retentionSize` for Prometheus at about 85% of the PVC; Loki
  `retention_period` plus compactor deletion; Tempo `max_block_bytes` / retention sized to its PVC;
  `BaukitPvcFillingUp` covered by kube-prometheus-stack defaults and tested in O4.
- [x] **R7 redis-ha recovery** (F-12): an init step asks Sentinel for the current master and only
  bootstraps `redis-0` as master when no Sentinel answers. Sentinels set `announce-hostnames` and
  `resolve-hostnames`. e2e test: kill the master, wait for failover, restart the old master, assert it
  rejoins as a replica with the data intact.
- [x] **R8 Prune safety** (F-08): drop `hcloud/namespace.yaml`. Annotate stateful objects (Namespaces
  that hold PVCs, CNPG Clusters, PVCs) with `kustomize.toolkit.fluxcd.io/prune: disabled`. The V4
  check enforces it.

## Phase S: supply chain and access

- [ ] **SC1 OCI charts with verification:** move each HelmRepository to `OCIRepository` plus `chartRef`
  where upstream publishes OCI charts (flagger is cosign-signed on ghcr; check prometheus-community,
  cnpg, kured, traefik, kyverno, falcosecurity). Add `verify.provider: cosign` with
  `matchOIDCIdentity` wherever signatures exist. Record unsigned ones in the base README. Update
  `validate.sh` and the V2 schema generator to pull OCI charts.
- [ ] **SC2 Digests everywhere** (F-25): use the charts' `image.digest` values, a CNPG `ImageCatalog`
  with digests, and `@sha256` on raw manifests. Add a V5 policy that fails any rendered container image
  without a digest outside an allowlist with reasons.
- [x] **SC3 Image scanning:** a `platform-images` gate step extracts every image from the rendered bases
  (V3 already renders them) and runs `trivy image --severity HIGH,CRITICAL --ignore-unfixed` with a
  checked-in `.trivyignore` whose entries need an expiry date and a reason. Also add
  `trivy config` over `deploy/` Dockerfiles and platform-infra's Terraform.
- [x] **SC4 Renovate:** custom managers for `tools.toml`, digests in raw manifests and the k3s image
  strings. Set `minimumReleaseAge` for charts and grouping per base. Configure the same in platform-infra
  (its `v*` tag pattern fix lands in PI1).
- [x] **S3 Traefik edge** (F-27, F-26): a default `TLSOption` (min TLS 1.2, `sniStrict`, curve
  preferences), a web to websecure redirect, a security-headers Middleware (HSTS, nosniff, frame deny,
  referrer policy) attached on websecure, rate limits keyed on `X-Forwarded-For` depth when behind a
  trusted LB, `global.checkNewVersion: false`, and access logs that drop query strings. e2e asserts
  TLS 1.1 handshakes fail and headers are present.
- [ ] **S4 Flux multi-tenancy:** the lifecycle patches the Flux controllers with
  `--no-cross-namespace-refs`, `--no-remote-bases` and `--default-service-account=default`, and enables
  notification-controller. `components/tenant` creates a tenant ServiceAccount with a namespace-scoped
  RoleBinding, which tenant Kustomizations must use (K4 policy). Tenant SOPS uses a per-tenant age
  recipient. e2e test: a tenant Kustomization that tries to write into `flux-system` fails.
- [ ] **S5 k3s hardening** for the lifecycle and the e2e cluster (and later the Terraform cloud-init):
  - `--secrets-encryption`;
  - an audit policy with a log file that Alloy ships to Loki;
  - `--pod-security-admission-config-file` with cluster default `enforce: baseline`,
    `audit/warn: restricted` and an exemption only for `kube-system`, so an unlabeled namespace is
    never privileged;
  - `--protect-kernel-defaults` if the WSL2 kernel tolerates it (record the result);
  - in Terraform, download the k3s installer by checksum instead of `curl | sh`.

  Run kube-bench (k3s CIS profile, pinned image) as a report-only e2e step and record the score in
  the log.
- [x] **S6 Local lifecycle exposure** (F-09, F-10): build the snapshot from
  `git ls-files --cached --others --exclude-standard`, bind the snapshot server and the k3d API, 80 and
  443 to `127.0.0.1` (or the k3d network gateway), run the snapshot image as non-root, validate
  `Content-Length` in `local-git-http.py` (with a test), and print secret locations instead of values
  in `status`.

## Phase PI: platform-infra adoption

Starts after the baukit release that carries phases V, E, C, K, N, O and FA (R and S items that are done
by then ride along).

- [ ] **PI1 Re-pin and unbreak** (F-13): pin `GitRepository/baukit` to the new `v*` tag. Replace the
  removed keycloak base with a platform-infra-owned component, or drop Keycloak from the overlay if the
  products no longer need it (confirm with the user). Fix the Renovate tag pattern.
  `./validate.sh` must be green.
- [ ] **PI2 Validate against the pin, not HEAD:** platform-infra's `validate.sh` creates a `git worktree`
  of baukit at the pinned tag (fails if the tag is missing), then runs baukit's
  `deploy/platform/validate.sh --target <overlay dir>` (V-phase scripts take target directories) so it
  gets schemas, `kyverno apply`, conventions and budgets for free. It adds `tofu fmt -check`,
  `tofu validate`, `tflint`, `trivy config terraform/`, `gitleaks` with a config, `shellcheck`, and the
  existing SOPS proofs. Add a `make validate` target and a pre-push hook script (opt-in).
- [ ] **PI3 Compose the hardened bases locally:** kyverno, kyverno-policies, falco, the platform
  observability pack, and the reusable components instead of the hand copies (`foundation/minio.yaml`,
  `issuers/`, the observability resource patches). Drop namespace manifests that duplicate base-owned
  namespaces (two owners with prune on). Migrate MinIO to the baukit RustFS component.
- [ ] **PI4 Tenants:** rebuild `apps/local/leitbild-*` on `components/tenant` with per-tenant service
  accounts, `targetNamespace`, a per-tenant age recipient, and `postgres-client`/`ingress-backend`
  namespace labels for the NetworkPolicies.
- [ ] **PI5 Alerting:** a SOPS-encrypted Alertmanager config Secret with a real receiver and an external
  dead man's switch for Watchdog. Flux `Provider`/`Alert` for reconcile errors. Fire a test alert and
  record its arrival in the log.
- [ ] **PI6 Edge and identity:** bind to `127.0.0.1`, set Keycloak `sslRequired: external`, put the
  `/admin` route behind an `ipAllowList` middleware plus the security-headers middleware.
- [ ] **PI7 SOPS hygiene:** add a second, offline break-glass recipient per environment, drop
  `mac_only_encrypted`, fix the rotation runbook (it points `SOPS_AGE_KEY_FILE` at the new key before
  `updatekeys`), rotate once, and write the cadence into the runbook.
- [ ] **PI8 Collapse testing and production** into tenant namespaces on the one local cluster, matching
  the local-first decision. Freeze the Hetzner overlays as a single overlay with postBuild substitution,
  validated but not bootstrapped. Apply S5 to the Terraform k3s config.
- [ ] **PI9 Live proof on the persistent cluster:** run the baukit e2e chainsaw suites (admission,
  netpol matrix, falco detection, alert pipeline) against `platform-local` in a dedicated test
  namespace, and add `docs/runbooks/` entries for policy triage, Falco triage, Flux stuck, age-key loss
  and local cluster loss.

## Verification per change

| Change touches | Run |
|---|---|
| any `deploy/**` file | `scripts/ci-gate.sh --step platform-validate`, `--step observability-rules`, `--step metric-names`, `--step observability-tests` |
| `deploy/chart/**`, `templates/backend/deploy/**` | the above plus `--step scripts-tests`, CLI snapshot bless and diff, `FIXTURE_FLAVORS="backend platform" make generated-fixture-gate` |
| policies, NetworkPolicies, Falco, alert routing, any base | the above plus `make platform-e2e` (under the host locks) |
| `cli/src/check/observability/**` | `--step cli-tests`, `--step baukit-check` |
| tool pins | `scripts/install-deploy-tools.sh` from a clean `.tools/`, then the static steps |
| before a release | full `scripts/ci-gate.sh` under the locks, plus `make platform-e2e`, `scripts/check-version-coherence.py` |

## Suggested execution order and parallelism

1. V1, V2 (one agent each), then V3 to V6 (two agents: `validate.sh` stages, and the convention checker
   with CI wiring).
2. E1 to E4 (one agent), in parallel with C1 to C5 (one agent, owns `deploy/chart`, `templates/`,
   `scripts/test_chart_*`) and with the F-defect fixes R1, R2, R8, O1, S6 (one agent, owns the affected
   bases).
3. K1 to K6 (one agent owns `kyverno*`), N1 to N5 (one agent owns `components/network-policy` and the
   per-base `network-policy.yaml` files), O2 to O7 (two agents: pack plus lints, then alerts, dashboards
   and runbooks), FA0 (spike).
4. FA1 to FA5 after FA0, R3 to R7, SC1 to S5.
5. Release, then PI1 to PI9 in platform-infra.

Directory ownership must stay disjoint inside each step. Agents that touch the same base serialize.

## Log

- 2026-11-14: plan written from a read-only audit of baukit `deploy/`, the templates, CI wiring and
  platform-infra. Verified on this host: BTF present for Falco `modern_ebpf`; kyverno, chainsaw,
  promtool, trivy and yamllint not installed; platform-infra pins a nonexistent tag and references the
  removed keycloak base; `ci.yml` has no platform-validate job. Upstream versions seen today: Kyverno
  chart 3.10.0 (v1.20.0), kyverno-policies 3.10.0, Falco chart 9.2.2 (Falco 0.45.2), falcosidekick
  chart 0.14.3.
- 2026-11-14: orchestration started. Integration branch `task/platform-hardening`
  (`baukit-worktrees/ph-int`); agents branch from it into `baukit-worktrees/ph-<task>`. Wave 1 (6 agents,
  Opus low): V1, V2, C (C1-C3, C5, C4 namespace part), D (R1, R2, R8, O1, S6), FA0 spike, K (K1-K5
  offline). Decisions D1-D9 taken at their defaults.
- 2026-11-14: V2 merged. kubeconform now checks 560 objects (was 92 checked, 95 skipped) including every
  HelmRelease's rendered chart, with no skip list and no `-ignore-missing-schemas`. CRD schemas come from
  the rendered charts (cert-manager, cnpg, kyverno and kube-prometheus-stack template their CRDs, so a
  tarball scan misses them) plus `flux install --export`. The CRD kind's own schema is built from
  yannh/kubernetes-json-schema `_definitions.json` at a pinned commit and sha256. Re-verified: step green,
  each misspelled fixture fails on its field. Follow-up for V1: the Flux pin currently lives in
  `platform-lifecycle.sh`; `tools.toml` must become the single source.
- 2026-11-14: FA0 spike done (report kept in the orchestrator state dir, facts copied here). Falco chart
  9.2.2 / Falco 0.45.2, falcosidekick 2.35.1, container plugin 0.7.6. Kubernetes metadata enrichment works
  through `collectors.containerEngine.engines.cri.sockets: [/run/k3s/containerd/containerd.sock]` (docker,
  containerd and podman engines off). WSL2 cannot attach the TOCTOU mitigation tracepoints; detection
  still works. Idle host noise: 4 events in 9.5 min, all without `k8s.ns.name` (host dockerd, other k3d
  nodes). Filter: a `baukit_cluster_pod` macro (`k8s.ns.name` present), but Hetzner must keep
  `container.id=host` events. Idle footprint: falco 22-85m CPU, 254-321Mi; sidekick 1m/15Mi (1 replica is
  enough). Metrics: `falcosecurity_scap_n_drops_total` for drops; `rules_matches_total` carries a numeric
  Falco priority (Critical+ is 0-2) and no namespace; sidekick `falco_events_total` has `k8s_ns_name` and
  `priority_raw` (its numeric priority runs the other way, so alert on `priority_raw`). Sidekick
  Alertmanager output needs endpoint `/api/v2/alerts`; Loki output needs `grafanaDashboard` off.
  `falco -V <file>` (one `-V` per rules file) works in the digest-pinned image with `--network none`.
  Agents now use private KUBECONFIG files after parallel k3d runs rewrote `~/.kube/config`.
- 2026-11-14: V1 merged. `deploy/tools.toml` pins kubeconform 0.8.0, kustomize 5.8.3, helm 4.3.1, kyverno
  1.20.0, chainsaw 0.2.15, promtool 3.15.1, pint 0.89.4, trivy 0.76.1, k3d 5.9.0, flux 2.9.4, kubectl
  1.36.6 and yamllint 1.38.0 (PyPI wheels with `--require-hashes`), plus the falco and prometheus images by
  digest. Spot-checked helm, kubeconform and kyverno sha256 against upstream checksum files. Flux is
  pinned only in `tools.toml` now (validate.sh and the lifecycle read it). `check-rules.sh` lost both its
  PATH and Docker fallbacks. Re-verified on integration: clean install, 17 installer tests, platform-validate,
  observability-rules, shell-lint, python-lint, workflow-lint green. Open: the gate steps exit 2 on a
  checkout without `.tools/`; V4/V6 make the steps install first.
- 2026-11-14: K1-K5 merged. D4 resolved: Kyverno v1.20 no longer runs legacy `kyverno.io` policies (the
  3.10.0 chart ships a `pre-install-check-legacy-policies` hook and an `acknowledgeLegacyPoliciesNotEnforced`
  switch), so every policy is `policies.kyverno.io/v1` (`ValidatingPolicy`, `ImageValidatingPolicy`,
  `GeneratingPolicy`) and the old `kyverno/policies.yaml` is gone. K1: ServiceMonitors on all four
  controllers, PDBs `maxUnavailable: 1`, exceptions only in `kyverno`, webhook timeout 5 s, webhooks skip
  kube-system, flux-system and kyverno, Policy Reporter 3.11.2 (metrics on, UI off). K3 deviation: the
  upstream PSS chart (3.10.0, restricted, Audit) has fixed policy names and only per-policy action
  overrides, so it cannot Enforce on tenant namespaces only; tenants get enforcement from PSA
  `enforce: restricted` (set by `components/tenant`) and `require-psa-labels` rejects a tenant namespace
  without it. K4: "Audit everywhere, Enforce on tenants" policies are split into two policies each. K5:
  `generate-tenant-defaults` (default-deny, DNS egress, LimitRange; synchronize + generateExisting).
  Re-verified: `kyverno test` 78/78, platform-validate (642 objects), shell-lint, python-lint. Known gaps:
  `verify-baukit-images` has no signed pass fixture (D6); `kyverno apply` cannot evaluate
  `unique-ingress-host-and-path` offline (cluster lookup), `kyverno test` covers it; expected `skip` rows in
  `kyverno test` never bite. Ordering constraint: `require-psa-labels` is Deny, so the e2e overlay must not
  compose `kyverno-policies` before N1 labels every namespace. Live check on k3d (Kyverno only): unlabelled
  namespace denied, tenant defaults generated, cluster-admin binding denied. One run hit the chart's
  legacy-check hook Job deadline on a slow pull; watch in e2e.
- 2026-11-14: V4 merged. `check_conventions.py` has 10 named rules, each with fail and pass fixtures (45
  tests), and runs as a validate.sh stage. YAML parsing is shared in `lint/helm_yaml.py`. Active rule set
  lives in `lint/conventions.toml`; rules today's bases break are pending on the task that fixes them, and
  the checker fails when a pending rule starts passing (so it has to be activated) or a rule is in neither
  list. Pending at merge: psa-labels 12 (N1), default-deny-network-policy 17 (N2), metrics-enabled 9 (O3),
  alert-tested 13 (O4), alert-metadata 1 and alert-runbook 15 (O6), rules-selected 2 (O1, F-02),
  prune-disabled 7 (R8). PolicyException budget 0, and the budget rule also fails when the count drops
  below it. Gate steps `platform-validate` and `observability-rules` install their pinned tools first, with
  a test that ties each `--only` list to the step's tools. Sent back once: metrics toggle detection moved
  from regex to parsed values, `psa-labels` now requires `enforce-version`.

- 2026-11-14: C merged (C2, C3, C5 done; C1 chart side done, the Traefik reachability test waits on E's
  `e2e/tests/ingress/`; C4 PSA half done). Images take `tag` or `digest`, `latest` fails the render,
  `values.schema.json` closes the top level, and each release gets its own ServiceAccount, startup
  probes, soft spread, a preStop sleep, the default PDB, job TTLs and an `/tmp` emptyDir. The smoke now
  creates the product namespace `restricted` before applying anything, runs dependencies in
  `<namespace>-deps` at `baseline`, and fails on PodSecurity events. Sent back once: the first version
  labelled the namespace only after non-compliant dependencies were running. Open for C4: the smoke
  applies `BAUKIT_SMOKE_POLICY_DIR` but does not install Kyverno itself, so the Kyverno half still
  needs a smoke install of `deploy/platform/kyverno` plus the policy library in Enforce mode. Breaking
  for product smokes: their dependency manifests now land in `<namespace>-deps`, so their Secrets must
  point at `<svc>.<namespace>-deps.svc`.
- 2026-11-14: Integration branch fix c9c106f8. V2 and V4 had added three `noqa` sites and pushed
  `baukit check` past the suppression budget. The lint modules now run subprocesses through
  `scripts/process_runner.py` and load `helm_yaml` with importlib, so `baukit check` is clean again.
- 2026-11-14: O2 merged. New chart `deploy/observability-platform/` (baukit-platform-observability,
  in release-train and version coherence) globs alerts, recording rules and dashboards into one object
  per file, switched per component; an unknown component or a missing `runbookBaseUrl` fails the render.
  Rule files carry a `RUNBOOK_BASE_URL/<slug>` token the template replaces, because `tpl` breaks on
  Prometheus `{{ $labels }}`. Skeleton alert `BaukitPlatformTargetDown` joins `up == 0` to
  `kube_namespace_labels{label_app_kubernetes_io_part_of="baukit-platform"}`; kube-state-metrics now
  allowlists that namespace label. `check-rules.sh` takes pack dirs and runs promtool check, promtool
  test and pint (`--fail-on warning`, `lint/pint.hcl`), and fails when a rule file has no test. The app
  pack templates use the same glob (F-53). Metric lint in Python and Rust gained per-pack allowlists
  (`catalog = false`) and stops counting LogQL stages; shared vectors in
  `fixtures/observability/metric-references.json`. One per-site pint disable on `BaukitDeadMansSwitch`
  (a heartbeat must always fire).
- 2026-11-14: O6 merged. Both packs require `runbookBaseUrl` and replace a `RUNBOOK_BASE_URL/<slug>`
  token; 15 runbooks (13 app, 2 platform) with Meaning, Impact, First checks, Fix, Escalation.
  `alert-metadata` and `alert-runbook` are active, and slugs resolve per pack (`docs/runbooks/app` vs
  `docs/runbooks/platform`), with tests for a missing file and a file in the wrong pack. From O4:
  `BaukitDeadMansSwitch` deleted (Watchdog covers it), the template's `ApiHighErrorRatio` is
  `severity: critical` with a description and a repo-relative runbook (`docs/runbooks/api-high-error-ratio.md`
  ships in the product). Every new alert must ship its runbook in the same change.
- 2026-11-14: FA1-FA3 merged. `deploy/platform/falco/`: chart 9.2.2, Falco 0.45.2 and falcosidekick
  2.35.1 by digest, `modern_ebpf`, the k3s CRI socket, falcoctl off (the image ships rules and the
  container plugin, and falcoctl would need internet egress), sidekick with Loki and Alertmanager
  (`/api/v2/alerts`, min warning) outputs. `rules/baukit.rules.yaml` lands in `/etc/falco/rules.d` via a
  ConfigMap, plus an optional `falco-overlay-rules` ConfigMap; `validate-rules.sh` runs `falco -V` in the
  pinned image with `--network none` inside platform-validate, and a broken macro fails the step.
  `validate.sh` now also passes `--api-versions monitoring.coreos.com/v1`, which renders the sidekick
  ServiceMonitor (the only new monitor). Five alerts, promtool tests, `dashboards/falco.json`, five runbooks.
  Live on k3d: rules loaded, the shell, unexpected-process and write-below-/etc rules fired with pod
  metadata; sent back once to prove the runc exclusion on the token rule live (0 events at container
  start, a real token read still fires). Idle footprint about 2 min after start: falco 255m/253Mi,
  sidekick 1m/14Mi. Known limits: "unexpected process" means "not PID 1"; no Flux noise macro yet
  (FA4 soak decides); network policies sit inline until N2's components land.
- 2026-11-14: O4 merged except the kube-prometheus-stack `defaultRules` tuning (waits until D, N1 and N2
  stop editing that file). 25 alerts in one file per component (flux 3, cert-manager 4, cnpg 6, traefik 2,
  kyverno 4, observability 4, kured 2), each with a firing and a quiet promtool case and a runbook; metric
  names traced to source at the pinned tags (sources in the O4 report), Traefik and Alloy names scraped
  live. Deviations with evidence: Traefik v3.7.13 has no reload-failure metric (live scrape), so that
  alert needs a LogQL rule and the pack's `loki-rules/` has no template yet (open, goes with O5); kured
  1.23.0 exports only `kured_reboot_required`, so "blocked" means reboot required and the node cordoned
  for 1h. Kyverno alerts use `kyverno_validating_policy_results_total` (CEL policies), and the enforced-
  violation alert joins Policy Reporter's `policy_report_result` to policies seen in `Deny` mode. The Flux
  alerts expect O3's kube-state-metrics config to emit `gotk_resource_info` with the flux2-monitoring-
  example labels (`exported_namespace`, `name`, `ready`, `suspended`, `customresource_kind`).
- 2026-11-14: O7 merged. Dashboards for flux, cert-manager, traefik, kyverno (Kyverno and Policy Reporter
  rows) and a platform overview (`Baukit*` ALERTS by component, Ready state from `gotk_resource_info`, PSA
  denials from `pod_security_evaluations_total`, `policy_report_result` failures). `cnpg.json` is the
  upstream cloudnative-pg/grafana-dashboards dashboard at cececeb3 (cluster-v0.0.5, Apache-2.0), changed
  only in datasource and uid; source and sha256 in the pack README. Its ConfigMap is about 252 KB, so it
  needs server-side apply (Flux and Helm are fine, client-side `kubectl apply` is not).
  `deploy/observability/lint/check-dashboards.py` checks structure in both packs inside `metric-names`
  (15 tests); the metric lint already covers every panel, proven by an injected unknown metric failing it.
  The Rust check accepts Grafana's `__expr__` datasource uid. Panels on controller_runtime metrics show
  data once O3 scrapes the CNPG operator and Flux controllers.
- 2026-11-14: Host sharing changed. Other projects on this host now share two heavy-run slots
  (`/tmp/claude-heavy-slots/slot-{1,2}`). D9 and E1 now say the platform e2e holds only `k3d-full.lock` and
  waits while both slots are taken; it no longer holds the android and browser locks, which blocked other
  projects' gates for the whole run.
- 2026-11-14: R7 merged. Both redis-ha containers start from `redis-ha.sh` (ConfigMap `redis-ha-scripts`,
  POSIX sh, every redis-cli call bounded by `timeout 3 redis-cli -t 2`). A starting pod asks the peer
  Sentinels for the master and joins it as a replica; only `redis-0` bootstraps, after three unanswered
  rounds 5 s apart. If Sentinels name the pod itself, it waits up to 60 s for a failover and exits 1 when
  they disagree. Sentinels monitor the master the local Redis chose, set `resolve-hostnames` and
  `announce-hostnames`, and announce the headless FQDN; both containers have a 3-minute startup probe.
  12 unittest cases (stubbed redis-cli) run in platform-validate. The chainsaw test
  `e2e/tests/redis-ha/` (kill master, promotion, old master rejoins as replica, key on both) passed twice
  on k3d, and a full StatefulSet delete came back with `redis-0` as master. Cold start is about 10 s slower.
- 2026-11-14 (D merged). R1: Traefik runs as a Deployment (1 replica, PDB `maxUnavailable: 1`,
  `maxUnavailable: 0`/`maxSurge: 1` rollout, `platform-critical`) behind a `LoadBalancer` Service that k3s
  ServiceLB answers. hostPorts are gone and the hcloud CCM load balancer controller is off. R2: CNPG
  `enablePDB: false` at one instance, the restore Role scopes delete/patch to the restore cluster by
  resourceName, and `hcloud-volumes-delete` exists for `RESTORE_STORAGE_CLASS`. R8: `hcloud/namespace.yaml`
  dropped, prune disabled on namespaces, the CNPG cluster and the rustfs PVC. O1: rule, ServiceMonitor and
  PodMonitor namespace selectors use `app.kubernetes.io/part-of: baukit-platform`, and the Grafana sidecar
  searches all namespaces with folder annotations. hcloud CCM and CSI keep their monitors disabled; O3 owns
  that. S6: snapshot from `git ls-files`, every bind on loopback or the k3d gateway, the snapshot server
  runs as uid 65532 and checks Content-Length. Convention rules `rules-selected` and `prune-disabled` are
  active. Local Grafana limit is 384Mi after an OOMKill at 256Mi. Live proof on k3d by the agent; I re-ran
  platform-validate, python-lint, shell-lint, observability-rules, metric-names and scripts-tests after the
  merge.
- 2026-11-14 (SC3, SC4 merged). `platform-images` (images lane, selected by path) renders the platform,
  scans 55 images and every tracked Dockerfile with trivy (HIGH/CRITICAL, fixable only, JSON output,
  no ignorefile) and compares each (target, ID) pair with `deploy/platform/lint/image-findings.toml`.
  The step fails on an unlisted finding, a listed ID trivy no longer reports, an entry with no findings
  left, and an expired or malformed entry. All entries expire 2026-11-28 and name SC2 as the follow-up:
  10 images where a bump fixes every ID, 25 where it fixes most, 15 already on the newest upstream
  release, and curl and posthog without a tag to compare. The trivy cache is `~/.cache/baukit/trivy`,
  a cold run takes about 19 min and a warm one about 2 min. After the merge, the D snapshot image
  (uid 65532) no longer reports DS-0002, the step failed on that stale entry as designed, and I removed
  the entry. Renovate gained custom managers for `deploy/tools.toml`, digests in raw manifests and the
  k3s pins, grouped per platform base, with `scripts/test_renovate_preset.py`. I also raised the android
  QA test stub deadlines (ADB 0.15 s to 10 s, watchdogs 5 s to 120 s), which had failed scripts-tests at
  load 70; three runs at load 36 to 77 passed.
- 2026-11-14 (N2-N4 merged). Components `default-deny`, `dns-egress`, `apiserver-egress`,
  `prometheus-scrape` and `node-cidr` in `deploy/platform/components/network-policy/`. Every overlay needs
  the `baukit-cluster-network` ConfigMap (local: nodeCIDR 10.89.0.0/16) and `platform-up.sh` pins the k3d
  subnet to `BAUKIT_PLATFORM_SUBNET` (default 10.89.0.0/24); `k3d_create_local_cluster` now forwards
  extra arguments to k3d. kube-router filters after DNAT, so apiserver egress is allowed to the node CIDR
  on 6443 and DNS only by the kube-dns pod selector; the `10.43.0.1:443` rule stays but grants nothing on
  k3s. Every base composes default-deny plus explicit allows, Falco included (sidekick to loki-gateway
  8080 and Alertmanager 9093, Prometheus to 8765 and 2801). PostHog: explicit pairs only, the plugin
  server alone reaches public 443. Its post-render patch now targets only `posthog-migrate-.*` (it
  failed on main because the grafana-annotation hook Job has no initContainers), and its PostgreSQL moved
  to 14.25 with `SKIP_SERVICE_VERSION_REQUIREMENTS=1` because PostHog 1.43.0 caps PostgreSQL at 14.1.0
  (this also skips its ClickHouse and Redis checks, both inside range today). Live matrix: 88 of 90 rows
  passed; #46 and #57 need N1's observability-agents namespace. Finding for N5 and the threat model:
  kube-router leaves a new pod unfenced for its first seconds (a connection 2 s after start got out in 2
  of 4 runs, at 12 s never), so the N5 runner waits 10 s before probing and `docs/threat-model.md` records
  the fail-open window. After the merge, `platform-images` flagged the PostHog PostgreSQL change: the 14.1
  entry went stale and 14.25 reports 26 Go stdlib findings in gosu, now listed with an upstream reason.
- 2026-11-15 VR (V5, R3) merged. `lint/ci-policies/` holds 12 offline ValidatingPolicies with 42
  `kyverno test` cases, and validate.sh applies them to every rendered base and overlay (35 HelmReleases,
  27 HelmRepositories, 8 Kustomizations; no OCIRepository exists yet, so that policy has no results). On
  review I added a guard: the stage fails when no Flux object produced a result, so an empty render
  cannot pass. Every HelmRelease has `driftDetection: warn`, `cleanupOnFail`, and `CreateReplace` where
  the chart ships `crds/`. Timeouts come from E's first live run at load about 70: kube-prometheus-stack
  227 s (12m), kyverno 161 s (9m), the rest under 90 s (5m). posthog keeps 20m, and hcloud and
  kyverno-policies are unmeasured at 5m. Not yet seen live: the falco and kyverno Helm tests with N2's
  policies in place.
- 2026-11-15 S3 merged. The `default` TLSOption sets TLS 1.2 minimum, `sniStrict`, curves and AEAD suites.
  web redirects to websecure, `default-security-headers` (HSTS, nosniff, DENY, referrer policy) sits on
  websecure, the limit middlewares key on the TCP peer (`depth: 0`), with the trusted-proxy overlay
  contract in the traefik README, and access logs drop the query string. Review found that redirect plus
  `sniStrict` made local hosts without a secret unreachable, so the local overlay now issues `localhost`
  and `*.localhost` from `local-self-signed` into the default TLSStore as a store certificate (`sniStrict`
  never falls back to `defaultCertificate`). Local hosts are one label under `localhost`. The chainsaw
  test `e2e/tests/traefik-edge` passed on a one-node cluster and fails against the old values. When E
  merges, `e2e-traefik` needs `dependsOn: e2e-issuer`.
- 2026-11-15 R6 merged. Prometheus `retentionSize` is 25GB on the 30Gi PVC and 1740MB on the local 2Gi PVC
  (Prometheus counts base-2). `lint/check_retention_size.py` renders every base, overlay and local component,
  and fails if the size is missing or above 90% of the PVC. Loki already had retention and the delete
  store, and now names the compactor working directory on its PVC. Tempo sets ingester `max_block_bytes`
  and the compactor flag through `extraArgs`, because the chart hardcodes that config section. A live
  `/status/config` shows the flag value. kube-prometheus-stack 88.6.5 keeps `kubernetesStorage` on, so the
  pack tests the upstream `KubePersistentVolumeFillingUp` from a verbatim copy in `upstream-rules/`.
  `lint/check_upstream_rules.py` fails when the rendered rule drifts from the copy. The retention budgets
  are arithmetic, not measured ingest, and are in the observability-stack README. O4 tuning must keep
  `kubernetesStorage` on, and must update the copy if it changes that rule's runbook URL.
- 2026-11-15 R5 offline part merged; R5 stays [~] until an integration e2e run passes
  `e2e/tests/backup-restore`.
  - The ScheduledBackup sets `immediate: true`.
  - The restore CronJob writes a sentinel row as the product owner (never superuser), takes its own plugin
    Backup, restores, and reads the token back over `kubectl exec` with the password on stdin. A missing
    or different token fails the job.
  - Each Backup is owned by its Job through the downward-API Job name and uid, so Job history limits and
    the TTL bound the Backup objects. CNPG v1.30.1 does not strip foreign ownerReferences.
  - Review rejected a first version that pruned with `delete` on all backups and relaxed the
    restore-role test. The Role is back to create and get, and the test is unchanged.
  - The README documents that the local default keeps backups inside the k3d node, and gives two
    off-node targets.
  - The netpol matrix needs `postgres-restore-test -> postgres-cluster-rw:5432 allow` when E merges.
