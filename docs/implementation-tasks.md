# Baukit implementation task list

**Source:** [shared-application-platform-analysis.md](./shared-application-platform-analysis.md) (sections 4, 14, 15)
**Orchestration:** Claude Code orchestrator delegating implementation to Codex subagents (`gpt-5.6-sol`, high reasoning), up to 3 in parallel.
**Status legend:** `[ ]` todo · `[~]` in progress · `[x]` done

This document is the single source of truth for what has been done and what remains.
The orchestrator updates it after every wave.

## Focus A: build baukit in this repository (current focus)

### Wave 1 — repository scaffold (sequential, 1 agent)

- [x] Repo skeleton per analysis §14: `rust/crates/*` (7 empty lib crates), `typescript/packages/*` (6 package stubs), `cli/`, `templates/`, `deploy/chart/`, `deploy/observability/{dashboards,alerts,recording-rules}/`, `agent-skills/`, `examples/`
- [x] Rust workspace: root `rust/Cargo.toml` with workspace dependency catalog per [compatibility matrix](./platform/compatibility-matrix.md) (Axum 0.8, SQLx 0.9, Utoipa 5, OpenTelemetry 0.32, config+dotenvy, tokio 1)
- [x] MSRV determined (stable − 2 per [conventions](./platform/baukit-conventions.md)) and recorded as `rust-version` in every crate
- [x] Repo hygiene: `.gitignore`, `rustfmt.toml`, workspace clippy lints, `deny.toml`, root `Makefile`, `README.md` seeded from conventions doc
- [x] CI: GitHub Actions workflow with fmt, clippy, test, MSRV check, cargo-deny; caching from the start

### Wave 2 — leaf crates (3 parallel agents)

- [x] `baukit-config`: layered config (defaults → optional local file → env with prefix convention) on `config`+`dotenvy`; redacted+zeroized secret wrappers; standard HTTP/ops/database/telemetry/shutdown field structs; startup validation with actionable errors; unit tests
- [x] `baukit-telemetry`: init per [telemetry spec](./platform/telemetry-spec.md) — JSON logs in deployed envs / pretty locally, OTLP traces, W3C propagation, resource attributes (`service.name`, `service.version`, `service.commit`, `deployment.environment.name`, `product`), Prometheus recorder, `build_info` gauge, explicit shutdown/flush; unit tests
- [x] `baukit-openapi`: standard Utoipa metadata + security schemes, deterministic schema serialization, schema-writer helper, CI drift-comparison utility; unit tests

### Wave 3 — composed crates (3 parallel agents)

- [x] `baukit-runtime`: graceful-shutdown token + drain timeout, task supervision, service identity/build info, API+ops listener composition helpers; unit tests
- [x] `baukit-http`: shared Axum layer stack — request IDs, trace context extract/inject, route-template request spans, panic handling, CORS/timeout/body-size/concurrency defaults, stable JSON error envelope (analysis §5.1), graceful drain, HTTP RED metrics recorded exactly once with spec §2.1 names/labels/buckets; unit tests
- [x] `baukit-ops`: separate ops router — `/healthz`, `/readyz` with extensible readiness registry + timeouts, `/metrics` Prometheus exposition, build/version info, optional SQLx pool metrics (spec §2.3); unit tests

### Wave 4 — test kit and reference example (up to 2 parallel agents, then verify)

- [x] `baukit-test`: PostgreSQL Testcontainers setup + migration lifecycle, test tracing init, health/readiness conformance tests, telemetry-spec §6 conformance test (scrape `/metrics`, assert names/labels, reject forbidden names), OpenAPI snapshot/drift assertions
- [x] `examples/minimal-api`: small Axum service composing runtime+config+http+ops+telemetry+openapi, passing the conformance suite; serves as living documentation
- [x] Cross-crate seam fix (orchestrator): `baukit-telemetry` recorder now applies spec §2.1 buckets to `http_request_duration_seconds` (`HTTP_DURATION_BUCKETS` const + test)
- [x] Full-workspace verification: `cargo fmt --check`, `clippy -D warnings` (all features), `cargo test` green across the workspace and examples/minimal-api (14 suites, 0 failures; Docker-based Postgres fixtures verified)

### Wave 4.5 — dogfooding friction fixes (from building minimal-api)

- [x] Unify `Environment`/`LogFormat`: `baukit-config` and `baukit-telemetry` expose separate enums requiring manual mapping — pick one canonical home (decide layering deliberately)
- [x] Make the `database` section of `BaukitConfig` optional — database-free services currently must carry and validate it
- [x] `baukit-http`: normalize Axum extractor/routing/method rejections into the standard envelope; provide 404/405 fallback handlers so products don't hand-roll them
- [x] `baukit-runtime`/`baukit-ops`: helper linking `TrafficGate` to `ShutdownToken` so readiness flips automatically during drain
- [x] `baukit-openapi`: make the bearer security scheme opt-in (unauthenticated services get an unused component today)
- [x] `baukit-runtime` composition docs: fix `.await??` example (actual nesting needs three `?`)
- [x] Document (don't "fix"): telemetry init is process-global and non-resettable — integration tests must consolidate into one process-wide contract test

### Wave 5 — TypeScript foundation (after Rust core is green)

- [x] pnpm/Turbo workspace wiring in `typescript/` (pnpm 11, Turbo 2, `packageManager` pinned, shared tsconfig/eslint/prettier)
- [x] `@baukit/analytics-core` per [privacy contract](./platform/analytics-privacy-contract.md): typed event schema, provider-neutral port, no-op + in-memory transports, consent state machine (unknown/granted/denied, drop-not-buffer), identity transitions (identify/alias/reset), allowlist + PII scrubber with mandatory tests, `schema_version` + common context, bounded buffering
- [x] `@baukit/analytics-posthog-web` and `@baukit/analytics-posthog-native` adapters
- [x] `@baukit/api-runtime`: base-URL/env resolution, token injection, request-ID/trace headers, normalized errors, idempotent-only retries, test transport
- [x] `@baukit/data-contracts`: transaction/pagination/atomicity contracts, contract-test suites, adapter helpers (Expo SQLite, Dexie, Node)
- [x] `@baukit/ui-tokens`: token schema (color/typography/space/radius/motion/elevation), CSS-variable + RN constant generation, validation

### Wave 6 — platform assets (required before product integration, per user 2026-08-08)

Order: 6a chart + observability (parallel) → 6b release engineering + CLI/backend template (parallel) → 6c frontend templates → 6d agent-skills → final verification.

- [x] 6a `deploy/chart/`: shared Helm application chart (API + ops listeners, migration job hook, worker, probes on ops port, no ops ingress)
- [x] 6a `deploy/observability/`: dashboards, recording rules, burn-rate alerts; lint job validating metric names against the telemetry spec
- [x] 6b Release engineering: release-plz config, Changesets, single release-train workflow bumping crates+packages+templates together
- [x] 6b `cli/` + `templates/` (backend): `baukit` CLI (`new` with `--backend`, `doctor`, `generate openapi-client`), `baukit.toml` manifest, backend product template per analysis §3, golden-snapshot tests, deterministic generation
- [x] 6c `templates/` (frontend): Expo mobile + Vite/TanStack web templates wired into `baukit new --mobile/--web`; generated-fixture CI matrix (backend/mobile/web/combined)
- [x] 6d `agent-skills/`: portable canonical skills invoking the CLI, installer into `.agents/skills` (Codex) and `.claude/skills` (Claude)
- [x] Final Focus A verification: full Rust + TS + example + generated-fixture matrix green

### Wave 7 — ship Focus A (per user 2026-08-08)

- [x] Commit and push baukit to the remote (32eb916; CI green at 31256104353 after deny/pnpm fixes)
- [x] Observe CI pipeline via codex subagent until green (fix failures if any)

### Wave 8 — platform gaps found while scouting Fitness Tracker

- [x] `baukit-runtime`: staggered shutdown — ops listener outlives the API during drain (opt-in)
- [x] `baukit-ops`: diagnostic (non-gating) readiness checks alongside gating ones
- [x] analytics: optional `clearPending` transport hook; PostHog adapters purge persisted SDK queues on consent denial
- [x] Push, CI green, cut first release-train tag `baukit-v0.1.0` (commit 0e1ff79, CI run 31256851784 green, tag pushed)

## Focus B: integrate into Fitness Tracker (after Focus A core)

Detailed integration map from the read-only scout (2026-08-08): FT is already on baukit's pinned versions (Axum 0.8.9, SQLx 0.9, OTel 0.32, Utoipa 5.5) — this is a contract migration. Waves per scout plan: B0 config/dependency bridge → B1 (runtime+telemetry+ops | HTTP errors+OpenAPI | analytics core) → B2 (domain metrics | consumers app/MCP | deploy/CI/dashboards) → B3 conformance gate.

- [x] B0: tagged baukit deps, `BaukitConfig<ProductConfig>` with legacy env aliases, `Secret<T>` adoption, single config load, rust-version 1.95
- [x] B1a: runtime/telemetry/ops — TelemetryBuilder, OpsRouter with diagnostic JWKS check, staggered drain, worker+migrate+seed process identity
- [x] B1b: HTTP errors/OpenAPI — baukit-http finalize, standard envelope (MCP dual-shape parser first), 404/405/extractor normalization, deterministic `backend/openapi.json`
- [x] B1c: analytics — @baukit/analytics-core with per-device tri-state consent, alias-once, scrub union, PostHog ≥4.62, queue clearing
- [x] B2a: product metric prefix `fittrack_`, DB acquire instrumentation, worker job metrics
- [x] B2b: consumers — app/MCP regenerated types, dual-shape error parsing removal plan, docs/AGENT_API.md
- [x] B2c: deploy/CI — release-job migrations, probes to ops listener, dashboards/Alloy `service` label, private git-dep auth, MSRV job
- [x] B3: integrated conformance gate — baukit-test suites + full backend/app/MCP/E2E verification

- [x] Adopt telemetry spec: duration metric plural → singular, log label `service_name` → `service` (covered by B1a/B2c)
- [x] Replace bespoke ops/telemetry/HTTP-metrics code with baukit crates (git deps pinned to tags) (covered by B0/B1a)
- [x] Adopt `@baukit/analytics-core` keeping existing consent/scrubbing behavior (covered by B1c)

## Focus C: integrate into OpenDialog (after Focus A core)

Done — verified on open-dialog `main` (commits e9db045..9ad098f, B0–B3 + green verification workflow).

- [x] Remove duplicate HTTP metric recording (custom RED middleware + `axum-prometheus` → exactly one recorder via `baukit-http`; conformance test asserts `axum_http_requests_total` is gone)
- [x] Migrate Figment → `baukit-config` (tagged git dep baukit-v0.1.0, no figment left in any Cargo.toml)
- [x] Adopt `@baukit/analytics-core` (vendored tgz in frontend/packages/analytics)

## Focus D: integrate into solo-leveling-system (after Focus C)

Scout summary (2026-08-08): SLS backend (`backend/`, 9 crates `sl-*`) already matches baukit's pins (Axum 0.8, SQLx 0.9, OTel =0.32, Utoipa 5, config+dotenvy, metrics 0.24.6, rust-version 1.97.1 > MSRV 1.95) — contract migration, no version bumps. Bespoke seams to replace: `sl-bin/{settings,telemetry,ops,shutdown}.rs`, `sl-api/error.rs`. HTTP RED metrics currently product-prefixed (`solo_leveling_http_requests_total`) — must become baukit spec §2.1 standard names; product prefix reserved for domain metrics. Frontend: single Expo app (`apps/mobile`) on **npm** (not pnpm), `packages/api-client` (generated from `backend/openapi.json`) + `packages/theme`; no analytics today. Deploy: Docker Compose + Dockerfile; CI: `ci.yml` (frontend) + `backend-ci.yml`. Work happens in the solo-leveling-system repo on branch `baukit-migration`.

### Wave D0 — foundation bridge (sequential, 1 agent)

- [x] Baukit git deps pinned to tag `baukit-v0.1.0` in `backend/Cargo.toml` workspace catalog; `BaukitConfig<ProductConfig>` replacing `sl-bin/settings.rs` bespoke loader with legacy env aliases so existing `.env`/compose files keep working; `Secret<T>` for JWT/SMTP/VAPID/AES secrets; single config load in `sl-bin`; keep rust-version 1.97.1

### Wave D1 — core seams (D1a ∥ D1c, then D1b — D1a/D1b overlap in `telemetry.rs`, cannot run concurrently in one worktree)

- [x] D1a runtime/telemetry/ops: `TelemetryBuilder` replacing `telemetry.rs` init, baukit-runtime shutdown (staggered drain, ops outlives API) replacing `shutdown.rs`, `OpsRouter` with gating Postgres readiness replacing `ops.rs`; process identity for api/worker/migrate/seed bins
- [x] D1b HTTP + OpenAPI: baukit-http layer stack (request IDs, spans, panic handler, RED metrics recorded exactly once with spec §2.1 standard names — bespoke `solo_leveling_http_*` middleware deleted), standard JSON error envelope in `sl-api/error.rs` with 404/405/extractor normalization, deterministic `backend/openapi.json` via baukit-openapi + drift test
- [x] D1c frontend api-runtime: vendored `@baukit/api-runtime` tgz consumed by `packages/api-client` (npm), regenerated typed client, request-ID/trace headers, normalized error parsing in the Expo app (tolerate old+new envelope during migration)

### Wave D2 — product surface (up to 3 parallel agents)

- [x] D2a domain metrics:
- [x] D2a-fix (orchestrator instruction error): worker job metrics must use spec §2.4 platform names `worker_job_runs_total{job, outcome}` / `worker_job_duration_seconds{job}` (unprefixed, `outcome` ∈ success|failure|retry) — D2a was told to prefix them `solo_leveling_` with `status`; baukit dashboards/alerts/lint query the spec names audit `solo_leveling_` prefix reserved for domain/worker metrics per spec, SQLx pool metrics via baukit-ops feature, worker job metrics
- [x] D2b deploy/CI: compose/Dockerfile healthchecks moved to ops listener, backend-ci SSH auth for private baukit git deps, `make gen-client` pipeline updated for deterministic OpenAPI, docs/CLAUDE.md command updates
- [x] D2c frontend envelope convergence: drop dual-shape error tolerance once backend envelope ships, Jest coverage kept ≥80%, theme audit + lint green

### Wave D3 — conformance gate (sequential, 1 agent)

- [x] Investigate/fix OpenAPI parameter misclassification flagged by D2c: 147 query-style params (`limit`, `cursor`, …) emitted as `in: path` in `backend/openapi.json` — likely D1b Utoipa annotation regression; fix at the source and regenerate schema + TS client
- [x] baukit-test conformance suites wired (ops + metrics + OpenAPI drift) and full CI-equivalent verification: backend fmt/clippy/tests/coverage (Docker-gated included), frontend lint/typecheck/test-coverage/build/e2e — all green on `baukit-migration`

## Log

- 2026-08-08: Task list created; Wave 1 dispatched to Codex.
- 2026-08-08: Completed Wave 1 scaffold and verification with MSRV 1.95; used reqwest 0.12.28 for `rustls-tls` and testcontainers 0.27.3 for modules compatibility.
- 2026-08-08: baukit-config implemented (codex) — layered loading, typed defaults, redacted secrets, and aggregate validation.
- 2026-08-08: baukit-openapi implemented (codex) — standardized metadata, error schemas, deterministic writes, and drift checks.
- 2026-08-08: baukit-telemetry implemented (codex) — correlated scrubbed logs, OTLP traces, Prometheus build info, and safe shutdown.
- 2026-08-08: baukit-http implemented (codex) — shared request lifecycle, safe errors, W3C tracing, and exact RED metrics.
- 2026-08-08: baukit-ops implemented (codex) — private ops routing, concurrent readiness, Prometheus exposition, and feature-gated SQLx metrics.
- 2026-08-08: baukit-runtime implemented (codex) — deadline-bound shutdown, supervised workers, shared identity, and graceful listener composition.
- 2026-08-08: examples/minimal-api implemented (codex) — in-memory notes API composing all six Baukit seams with contract tests.
- 2026-08-08: baukit-test implemented (codex) — PostgreSQL fixtures, JWT helpers, and operations, metrics, and OpenAPI conformance assertions.
- 2026-08-08: Wave 4.5 friction fixes (codex) — unified shared vocabulary in dependency-light `baukit-core` and removed dogfooding boilerplate across configuration, HTTP, runtime, OpenAPI, and telemetry tests.
- 2026-08-08: TS workspace wired (codex) — pinned ESM build, lint, format, test, Turbo, Make, and CI tooling across six packages.
- 2026-08-08: @baukit/api-runtime implemented (codex) — openapi-fetch runtime with explicit configuration, typed failures, safe retries, and MockFetch.
- 2026-08-08: @baukit/analytics-core implemented (codex) — typed consent-gated analytics with identity, privacy filtering, and bounded delivery.
- 2026-08-08: data-contracts + ui-tokens implemented (codex) — provider-neutral storage conformance and deterministic accessible token compilation.
- 2026-08-08: PostHog adapters implemented (codex) — lazy self-hosted web and native transports preserve core privacy and identity boundaries.
- 2026-08-08: observability pack implemented (codex) — portable RED dashboard, SLO alerts, and spec metric linting.
- 2026-08-08: shared Helm chart implemented (codex) — reusable process workloads with private ops, release hooks, and default-deny networking.
- 2026-08-08: baukit CLI + backend template implemented (codex) — deterministic conflict-safe generation with a compiling conformance-tested backend fixture.
- 2026-08-08: frontend templates implemented (codex) — composable Expo and Vite apps with privacy-safe Baukit integrations and fixture coverage.
- 2026-08-08: release engineering implemented (codex) — unified private train with coherent Rust, TypeScript, template versions and one tag.
- 2026-08-08: agent-skills implemented (codex) — four portable CLI-driven workflows with dual-harness installation.
- 2026-08-08: Focus A verification complete (orchestrator) — Rust workspace, example, CLI, TS turbo, lints, coherence, and a fresh combined fixture all green; committing and pushing.
- 2026-08-08: CI green on main (codex) — removed vulnerable RSA, allowed permissive licenses, and made pnpm's age gate reproducible.
- 2026-08-08: platform gaps closed (codex) — staggered drain, diagnostic readiness, and provider queue purging added.
- 2026-08-08: Focus B complete (orchestrator) — Fitness Tracker fully migrated onto baukit on branch baukit-migration (5 commits, B0-B3), all gates green with Docker tests mandatory.
- 2026-08-08: Focus C verified done (orchestrator) — open-dialog main carries B0–B3; figment/axum-prometheus gone, analytics-core vendored; checked off.
- 2026-08-08: Focus D scouted and planned (orchestrator); SLS branch baukit-migration created; @baukit/api-runtime tgz pre-built for D1c.
- 2026-08-08: Focus D shipped (orchestrator, per user) — Node bumped 20→24 (LTS; satisfies baukit engines, NPM_CONFIG_ENGINE_STRICT workaround removed) in both CI workflows + Dockerfile.e2e + README, `docs/baukit-setup.md` added (deploy-key ssh/gh setup, mirrors open-dialog/fitness-tracker); e2e re-run green on Node 24 (44/44). Commit 7a3b140 on baukit-migration, merged to main as 103bea7 and pushed.
- 2026-08-08: D3 complete (codex, commits e28d729/520d286/64c169e) — Focus D done. OpenAPI bug root-caused as a D1b regression: renaming utoipa-recognized extractor identifiers `Path`/`Query` to `ApiPath`/`ApiQuery` broke utoipa's axum inference, defaulting query DTOs to path params; fixed by aliasing back + `#[into_params(parameter_in = Query)]` + regression tests, artifacts regenerated (238 path / 147 query / 0 optional-path params, matches main). Baukit ops + telemetry §6 + OpenAPI drift conformance wired. Full matrix green: fmt/clippy, `cargo test --workspace -- --include-ignored`, backend coverage 361/361 (82.25%), make lint/typecheck, frontend coverage 528 tests (87.1%), build (191 routes), e2e Docker build + 44/44 Playwright, gen-client no-op. JWT crypto provider pinned (baukit-test pulls a conflicting jsonwebtoken provider). Not pushed. New platform backlog: baukit-test crypto-provider conflict + second reqwest version; metrics conformance helper doesn't enforce worker metric families (product must assert them itself).
- 2026-08-08: D2a-fix complete (codex, commit a651da2) — worker metrics on spec §2.4 names/labels (`worker_job_runs_total{job,outcome}`), domain metrics untouched; fmt/clippy/tests green.
- 2026-08-08: D2b complete (codex, commit d87d4ef) — /readyz healthchecks on ops ports 19464/19465, BAUKIT_DEPLOY_KEY SSH auth across backend CI + frontend E2E, vendored tgz in E2E image, engine-strict=false for Node 20, deterministic gen-client, docs. Deferred to D3: authenticated Docker build + E2E (no SSH agent in agent shell), repo-wide lint, in-place gen-client.
- 2026-08-08: D2a complete (codex, commit 19bbe01) — domain metrics conformed and described, worker job metrics instrumented, readiness acquisition via baukit_ops::acquire; fmt/clippy/tests green. Codex correctly flagged a spec conflict: orchestrator's prompt contradicted telemetry-spec §2.4 on worker metric names → D2a-fix dispatched. Gap: baukit_ops::acquire can't instrument implicit PgPool executors/Pool::begin (7 sites listed in transcript).
- 2026-08-08: D2c complete (codex, commit 4336f32) — TS schema regenerated, legacy envelope parsing removed, envelope coverage added; typecheck/coverage (86.54%)/build green, repo-wide lint deferred to D3. Flagged: openapi.json emits 147 query params as path params → D3 item.
- 2026-08-08: D1b complete (codex, commit 9bc0590) — baukit-http layer stack + envelope (product codes preserved, SCIM keeps RFC 7644 shape), 404/405/extractor normalization, exact-once spec §2.1 metrics, deterministic openapi.json + drift test; fmt/clippy/tests green. Friction: fixed CORS header set needs product extension (Accept, x-webhook-secret); JSON extractor code `validation_failed` vs product `invalid_json`; template Cargo.toml diagnostics again.
- 2026-08-08: D1c complete (codex, commit 7abe09e) — @baukit/api-runtime vendored + adopted in api-client and Expo app, dual-envelope error tolerance, 401 refresh preserved; lint/typecheck/coverage (86.54%)/build green. Friction: baukit TS packages declare Node >=24 while product CI runs Node 20 (EBADENGINE); repo does per-scope npm installs, not one workspace install.
- 2026-08-08: D1a complete (codex, commit c16cefc) — baukit runtime/telemetry/ops adopted for api/worker/migrate/seed, staggered drain, gating Postgres readiness + pool metrics, /buildinfo; fmt/clippy/tests green. Friction: no complete OTEL_SDK_DISABLED switch (mapped to zero sampling); acquisition timing only via baukit's helper.
- 2026-08-08: D0 complete (codex, commit 59b3614) — BaukitConfig<ProductConfig> bridge with legacy env aliases, Secret<String>, shared loader across api/worker/migrate/seed; fmt/clippy/tests green. Friction noted for platform backlog: env parsing coerces numeric-looking secrets; private git deps needed `git-fetch-with-cli`; unrendered template Cargo.tomls emit parse diagnostics; exact `thiserror =2.0.20` pin advances consumer lockfiles.
