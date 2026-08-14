# Baukit implementation task list

**Source:** [shared-application-platform-analysis.md](./shared-application-platform-analysis.md) (sections 7, 8, 10, 14, 15)
**Orchestration:** Claude Code orchestrator delegating implementation to Codex subagents (`gpt-5.6-sol`, high reasoning), up to 3 in parallel.
**Status legend:** `[ ]` todo · `[~]` in progress · `[x]` done

This document is the single source of truth for what has been done and what
remains. The orchestrator updates it after every wave. Completed work is
archived verbatim: Phases 0–2 (Focus A–H) in
[implementation-tasks-archive-phase0-2.md](./implementation-tasks-archive-phase0-2.md),
and the completed Phase 3 / Focus RL waves (I0–I3, the local pivot waves
L1/L1b/LD/L2/L3 and M1–M5, I5a, RL1/RL2/RL4) in
[implementation-tasks-archive-phase3-rl.md](./implementation-tasks-archive-phase3-rl.md).

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
- **Local-first platform** (user decision 2026-08-09): one local k3d cluster
  (`platform-local`) hosts the full platform plus both app environments —
  `testing` and `production` as namespaces on the same cluster. The Hetzner
  path stays intact in platform-infra `terraform/` and resumes as Wave R only
  on an explicit user ask. Delivery is GHCR-backed (private packages, newest
  3 versions retained), every leitbild release rides a Flagger canary in both
  environments, and PostHog runs in-cluster as an optional flag-gated base.
- **Baukit commit freeze lifted** (2026-08-09): the orchestrator commits/pushes
  baukit and cuts releases (`scripts/release-train.sh`).
- Latest release-train tag: `baukit-v0.5.1`; leitbild and the platform-infra
  cluster remain pinned to `baukit-v0.5.0`. Fitness Tracker and
  solo-leveling-system remain on `baukit-v0.2.0`; OpenDialog is the first
  product moving to the direct pnpm Git packages from `baukit-v0.5.1`.

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

All locally implementable Focus I work is **done**: I0 → I3, the local-first
pivot waves L1/L1b/LD/L2/L3 (user decision 2026-08-09: no paid environments,
everything on one local k3d cluster), the pivot continuation M1–M5, and the
leitbild adoption I5a. The full task lists, I0 pin table, pivot narratives,
and wave logs are preserved verbatim in
[implementation-tasks-archive-phase3-rl.md](./implementation-tasks-archive-phase3-rl.md);
the table below is the summary. Former I4/I5b/I6/I7/I8/I9 were dissolved
2026-08-09: locally-testable items went into M1–M5, real-environment items
into Wave R (parked until an explicit user ask). I10 is an optional backlog,
not exit-blocking.

### Completed Focus I waves (2026-08-09 → 2026-08-10)

| Wave | Delivered | Exit evidence |
|---|---|---|
| I0 | Perishable-claim verification + component picks; pin list for every I1 base (Flux v2.9.4, k3s v1.36.3+k3s1, cert-manager v1.21.1, Traefik 41.2.0, kyverno 3.8.2, kured 6.1.0, hcloud CCM 1.34.0 / CSI 2.22.1, CNPG 0.29.0 + barman plugin v0.14.0, Keycloak operator 26.7.1, kube-prometheus-stack 88.2.0, Loki 18.7.6, Tempo 2.2.3, Alloy 1.11.1, external-dns 1.21.1) | analysis §17 entries 21–29; pin table preserved in the archive |
| I1 | 10 secret-free Flux-native platform bases under `deploy/platform/` on the I0 pins; `validate.sh` wired into `make ci` | validate.sh green (10 bases + "everything" overlay), metric lint, coherence, full `make ci` |
| I2 | Local k3d flavor (`overlays/local/` + `platform-up.sh`); fixture product deployed with the **unmodified** baukit-app chart against platform Keycloak + CNPG; template/chart fixes landed in the right layer | integration proof all-PASS (PKCE, CRUD, worker e2e, backup + 56 s restore); footprint recorded as node-sizing input |
| I3 | platform-infra repo: OpenTofu modules (Hetzner cx43 nodes, DNS, buckets, encrypted remote state), Flux entrypoints + overlays for 3 clusters, SOPS+age, Renovate preset, runbook skeletons | `tofu` fmt/validate/tflint/test green; repo `validate.sh` (kustomize + kubeconform + flux build); SOPS round-trip proven |
| L1 | Persistent local cluster `platform-local` on real GitOps from `github.com/PatrickKoss/platform-infra`; SOPS live with a real age key | 16/16 Kustomizations + 8/8 HelmReleases Ready from GitHub; self-healing proof |
| LD | Progressive-delivery decision: Flagger 1.44.0 with the direct Traefik provider, zero-customer-traffic pre-rollout gate, stepwise weights + auto-rollback | `docs/progressive-delivery.md`; 13-point manifest checklist handed to L2 |
| L1b | Reproducible lifecycle automation — generic `make platform-{up,down,nuke,recreate}` layer in baukit, thin identity wrapper in platform-infra | restart survival unaided in 275 s; nuke→recreate in 616 s purely from git + age key |
| L2 | leitbild `testing`/`production` as namespaces via real GitOps; registry-less `make release`/`make promote`; app-registration pattern documented as the template | both envs green; headless-PKCE + CRUD + worker e2e against platform Keycloak |
| L3 | Promotion loop proven twice on real merged PRs; four failure drills; deploy/promote runbooks written from what was done | PRs #1/#2 merged; api-kill, worker-kill, backup+restore, alert drills all pass |
| M1 | `baukit-v0.3.1` release train; cluster consumes the real GitHub tag via read-only deploy key; snapshot bridge retired to dev-only | tag live on-cluster (`baukit-v0.3.1@sha1:c292c3c`); 18/18 Kustomizations Ready |
| M2 | GHCR-backed delivery: process-specific scratch images (−73.8%), keep-newest-3 pruning, SOPS-encrypted pull secrets | full release→testing→promote→production round pulling from GHCR |
| M3 | TLS on the `local-ca` chain for 4 hostnames; ops-exposure probes; dead-man watchdog; restore-test staleness alert | `curl --cacert` green ×4; silence-means-page and staleness drills both proven |
| M4 | Flagger canary live for `leitbild-api` in both envs (`baukit-v0.3.2`); staging bridge deleted | good release promoted through the full ladder; bad release auto-rolled-back with zero customer impact |
| M5 | Optional flag-gated in-cluster PostHog base; releases `baukit-v0.3.3/4/5` | live privacy proof at the ClickHouse layer; footprint + enable/disable runbook recorded |
| I5a | leitbild 0.2.0 → 0.3.0: `@baukit/auth-web`, `baukit-jobs`, chart 0.3.0 features (net −271 LOC) | full gate green with zero unresolved metrics; friction logged as §11.4 research |

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
- [ ] **User action:** install the hosted Renovate GitHub App on baukit,
  platform-infra, leitbild, architecture-health-platform (and the three
  migrated products); confirm free-tier coverage for private personal repos
  (I0 verified the claim)
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

### Completed Focus RL waves (2026-08-10 → 2026-08-11)

| Wave | Delivered | Exit evidence |
|---|---|---|
| RL1 | `baukit-ratelimit` crate (`RateLimitStore` port, atomic Lua Redis adapter, in-memory adapter, identity+IP axum layer, 429 envelope, `http_rate_limit_decisions_total`); chart opt-in `redis.enabled`; observability pack entries; `deploy/platform/redis/` shared base | all gates green: tests `--include-ignored`, cargo deny, MSRV 1.95, metric lint, both repos' `validate.sh` |
| RL2 | Release train `baukit-v0.4.0` (41 files, six CLI golden trees refreshed) | full gate incl. 3-flavor generated fixture; tag pushed |
| RL4 | Sentinel HA: `redis+sentinel://` scheme + `connect_sentinel`, `start_redis_sentinel()` test kit, chart `redis.replicas` StatefulSet path, `deploy/platform/redis-ha/`; release `baukit-v0.5.0`; platform-infra on the HA base + v0.5.0 pins | live k3d failover proof: promotion 6.7 s, write on new master 7.0 s, 3/3 rejoin 35.6 s |

Full RL1/RL2/RL4 task lists and log entries are in
[the archive](./implementation-tasks-archive-phase3-rl.md).

### Wave RL3 — leitbild reference integration + live proof (1 agent)

- [x] Leitbild backend: bump all baukit git deps to `baukit-v0.4.0`; wire the
  limiter (identity low, IP very high — values chosen in leitbild config per
  env); log migration friction 0.3.2→0.4.0 in leitbild docs
- [x] Leitbild deploy: chart `redis.enabled`, `<PREFIX>__RATE_LIMIT__…` env in
  HelmRelease values, compose.yaml redis for local dev, Traefik middleware
  raised to coarse-net levels (kept as outer safety net)
- [x] Leitbild full gate green (`make check`, tests `--include-ignored`,
  render-gitops) + platform-infra pin bump `baukit-v0.4.0` + `validate.sh`
- [~] Live proof on local k3d (codex verifier, 2026-08-11): canary promoted on
  chart 0.5.0 + release `083b5343796f`; identity hammer (900 req, conc 50)
  → 709×200 then 191×429 with `Retry-After: 1` + `RateLimit-Remaining: 0`
  (limit 700 = 600+100 burst); unauth flood advanced the `scope="ip"` counter
  710→1710 (limit 13000); Prometheus shows
  `http_rate_limit_decisions_total{scope="identity",outcome="limited"} 191`.
  **Open**: A-429-while-B-200 isolation pair — testing realm has only one
  identity (`wave-l2-smoke`); creating a temp Keycloak identity was blocked
  by the session permission classifier, needs user go-ahead
- [x] Platform Redis base live on local k3d (decision 8): proven at v0.4.0
  (Kustomization Ready, pod Ready, NetworkPolicy allow + both deny cases) and
  re-proven at v0.5.0 on the `redis-ha` base (RL4d live proof)
- [~] Orchestrator gate: leitbild + platform-infra gates re-verified via the
  RL4c/pin-bump codex runs (all green incl. `--include-ignored`); live-proof
  reviewed from the codex report — pending only the identity-isolation pair
  above

## Log

Wave-log entries for all completed waves (I0–I3, L1/L1b/LD/L2/L3, M1–M5, I5a,
RL1/RL2/RL4) are archived verbatim in
[implementation-tasks-archive-phase3-rl.md](./implementation-tasks-archive-phase3-rl.md).

- 2026-08-14: **Git-path TypeScript package distribution fixed and
  `baukit-v0.5.1` released.** A root pnpm marker and lockfile make pnpm prepare
  subdirectory packages with the nested TypeScript workspace instead of
  falling back to npm and rejecting `workspace:*`. Direct external installs
  of analytics core plus both PostHog adapters were built and imported; the
  full local CI mirror, Docker-gated Rust suite, dependency policy checks,
  observability lint, and combined/auth generated fixtures passed.

- 2026-08-11: **Wave RL3 live proof nearly done (codex verifier on k3d
  testing).** Leitbild pinned to baukit-v0.5.0 (16 files, scoped lock
  updates only — 11 baukit crates + `@baukit/*` pins; commit 083b534),
  released `083b5343796f` to testing (f8d7723), Flagger canary
  20→50→Promoting→Succeeded in ~2.5 min, redis pod healthy with
  `runAsUser: 999`. Live: `/me` via PKCE (`leitbild-web`,
  realm `leitbild-testing`); baseline 200 with
  `RateLimit-Limit: 700` / `Remaining: 699` / `Reset: 60`; hammer 900 req
  → 191×429 (`Retry-After: 1`, `Remaining: 0`, `rate_limited` envelope);
  IP scope: 1000 unauth req advanced `scope="ip",outcome="allowed"`
  710→1710 (`RateLimit-Limit: 13000`), no exhaustion flood per DoS cap;
  Prometheus confirms `identity/limited 191`. Remaining: two-identity
  isolation pair (A 429 while B 200) — realm has a single user and the
  permission classifier blocked launching the codex run that would create
  a temporary second Keycloak identity; awaiting user decision.
