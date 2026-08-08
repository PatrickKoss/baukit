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

- [ ] Remove duplicate HTTP metric recording (custom RED middleware + `axum-prometheus` → exactly one recorder via `baukit-http`)
- [ ] Migrate Figment → `baukit-config`
- [ ] Adopt `@baukit/analytics-core` (its typed interface is the model; add FT-style scrubber)

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
