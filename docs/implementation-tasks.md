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

## Focus E: Phase 2 platform gaps (roadmap §15 Phase 2, in this repository)

Focus A already delivered most of the Phase 2 tooling bullets (CLI `new`/`doctor`/`generate openapi-client`, backend/mobile/web templates, shared chart, observability pack, agent skills, generated-fixture CI matrix). What Phase 2 still lacks before the reference app can exist: an identity port (the exit criterion demands an *authenticated* CRUD app; today only `baukit-test/jwt.rs` exists), auth-capable templates with a local Keycloak, and the friction backlog accumulated during Focus B–D. Order: E0 ∥ E1 → E2 → E3 (parallelizable) → E4. Only E1+E2 block Focus F; E3 can run alongside F.

### Wave E0 — re-verify time-sensitive claims (1 agent, read/web only)

- [x] Re-check upstream claims the analysis flagged as perishable before locking anything further: TanStack Start release status (still RC? stays opt-in), Loki simple-scalable deprecation timeline, current Expo SDK vs template pin, PostHog self-host hobby baseline. Record deltas as appended decisions in analysis §17; adjust templates only if a claim materially changed.

### Wave E1 — `baukit-auth` identity port (1 agent)

- [x] New crate `rust/crates/baukit-auth` per analysis §5.2: OIDC/JWT verification port — issuer/audience validation, JWKS fetch with caching + rotation + timeout, alg allowlist; verified claims map to an internal `Principal` (subject, optional org/tenant context), never provider-specific claims beyond the boundary
- [x] Axum integration: extractor/middleware layered on `baukit-http` producing the standard envelope (`unauthenticated` 401 / `permission_denied` 403 codes), utoipa bearer security-scheme wiring via the existing `baukit-openapi` opt-in
- [x] Keycloak-shaped defaults (issuer URL convention, realm discovery via OIDC metadata); provider stays behind config — no Keycloak/Clerk SDK dependency in the crate
- [x] `baukit-test`: extend `jwt.rs` into a mock OIDC/JWKS server + token-minting fixture usable by product integration tests; auth conformance tests (expired/wrong-audience/wrong-issuer/unsigned tokens all yield the standard envelope). Fix the known jsonwebtoken crypto-provider conflict (Focus D backlog) here, where it originates

### Wave E2 — auth capability in CLI + templates (1 agent, after E1)

- [x] `baukit new --auth oidc` flag, recorded in `baukit.toml`; omitted means no auth scaffolding (omitted, not empty, per generator rules)
- [x] Backend template: protected-route example wired through `baukit-auth`, subject→internal-user mapping table + migration, config fields for issuer/audience
- [x] Template `compose.yaml`: Keycloak service with dev realm import (realm JSON in the template, seeded test user + confidential/public clients), so `docker compose up` yields a working local IdP
- [x] Web template: OIDC auth-code + PKCE login/logout/refresh (product-local code, not a shared package yet — the two-consumer guardrail applies); mobile template: `expo-auth-session` equivalent
- [x] Fixture CI matrix gains an `--auth oidc` flavor; golden snapshots updated; all flavors (backend/web/mobile/combined/auth) stay green including clippy on generated output

### Wave E3 — Focus B–D friction backlog (up to 2 parallel agents; does not block Focus F)

- [x] `baukit-test`: eliminate second reqwest version; metrics conformance helper optionally enforces worker metric families (spec §2.4) when the product declares a worker
- [x] `baukit-http`: CORS header extension point (products needed `Accept`, `x-webhook-secret`); configurable JSON-extractor error code (products wanted `invalid_json` over `validation_failed`)
- [x] `baukit-telemetry`: honest `OTEL_SDK_DISABLED` switch (currently mapped to zero sampling)
- [x] `baukit-ops`: document or extend `acquire` for implicit `PgPool` executors/`Pool::begin` (7 uninstrumentable sites found in Focus D)
- [x] `baukit-config`: stop coercing numeric-looking secrets during env parsing; relax the exact `thiserror` pin

### Wave E4 — release (sequential, orchestrator)

- [x] Version coherence (`scripts/check-version-coherence.py`), full `make ci` + fixture matrix + Docker-gated tests (`--include-ignored`), cut release-train tag `baukit-v0.2.0` — the tag Focus F pins (CI observation pending: GitHub Actions refuses all jobs account-wide, see Log)

**Focus E exit:** `baukit new x --backend --web --mobile --auth oidc` produces a product where a protected endpoint rejects/accepts tokens from the composed Keycloak, verified by generated tests, on a released tag.

## Focus F: reference app — journaling product `leitbild` (after E2)

Decisions from user (2026-08-08): journaling app first, architecture health platform rewrite follows as Focus G. All three targets (backend + web + mobile). MVP includes all four feature pillars: guided authoring program, free-form daily entries, AI reflection, reminders/streaks — staged below so the roadmap exit criterion (authenticated CRUD + telemetry, deployed < 1 h) is met at F8 regardless of how far F5/F6 have progressed.

Working name `leitbild` (German: guiding vision) — rename at F0 if the user supplies a product name. Lives in its own private repo `github.com/patrickkoss/leitbild`, scaffolded by the CLI, baukit consumed only via git deps pinned to `baukit-v0.2.0` — no path deps, no copied files; every friction point is product research and gets logged here.

**IP note:** the guided program is Future-Authoring-*inspired* (multi-stage guided life/goal writing). All prompt content is original; no text, stage names, or exercise structure copied from the Self Authoring Suite.

**Analytics privacy rule (non-negotiable, per privacy contract):** journal/prompt/response content never enters analytics, logs, or metrics. Events are structural only (`entry_created`, `program_section_completed`, …); the scrubber tests assert content fields are rejected.

### Wave F0 — scaffold + walking skeleton (sequential, 1 agent)

- [x] `baukit new leitbild --backend --web --mobile --auth oidc` against tag `baukit-v0.2.0`; private repo created; generated CI green **before any product code** (CI-equivalent run locally per the no-Actions decision; hosted run refused by account billing as expected)
- [x] Record scaffold friction (missing files, manual steps, doc gaps) in the Log — each manual step is a platform bug (17 items logged, see F0 Log entry)
- [x] Compose up (Postgres + Keycloak), login from web fixture page, hit protected endpoint — walking skeleton demo (401→200 on `/me` via PKCE login with seeded user; healthz/readyz/metrics green; clean teardown)

### Wave F1 — domain + persistence (1 agent)

- [x] Domain crates per template layout: `JournalEntry` (date, markdown body, tags, optional mood), `Program`/`ProgramStage`/`ProgramSection` (ordered, prompt text, guidance, optional word targets), `ProgramRun` (per-user progress, per-section responses, draft autosave with revision, resume, completion), streak facts
- [x] Program content is data, not code: versioned content files loaded by the seed binary; authoring the original guided-program content (stages like past-retrospective → present-faults/virtues → future-vision → concrete-plan, all original prose) is an explicit checklist item, not an afterthought ("Deliberate Direction v1": 4 stages, 16 sections, idempotent seed verified by double-run)
- [x] Migrations, ports, services, unit + property tests (ordering/resume invariants); release-job migration convention from the template kept intact

### Wave F2 — API + contract (1 agent, after F1)

- [x] Authenticated endpoints: entries CRUD with pagination/search, program catalog, program-run lifecycle (start / save-section / complete-stage / resume), user profile bootstrap (subject→internal user on first login)
- [x] Standard envelope, per-route authz, OpenAPI committed with drift test, `baukit generate openapi-client` output consumed by frontends (client regenerated twice with identical hashes)

### Wave F3 — web app (1 agent, after F2; ∥ F4)

- [x] Vite/TanStack Router/Query per template: PKCE auth flow (refresh support, Strict-Mode-safe callback), entries list/editor/search, guided-program flow (stage navigation, per-section writing screen with autosave + word count, progress indicator, resume, completion summary with markdown export)
- [x] `@baukit/ui-tokens` theme; `@baukit/analytics-core` events under the privacy rule above (privacy-allowlisted events, no-op transport until Phase 3 PostHog exists)
- [x] Vitest/Testing Library (8 tests) + Playwright critical path against the real stack, one command (`corepack pnpm test:e2e`): login → write entry → start program → complete a section → resume

### Wave F4 — mobile app (1 agent, after F2; ∥ F3)

- [x] Expo Router app: daily entries with local cache (Expo SQLite via `@baukit/data-contracts`, product-local SQLite adapter), program reading + section responses, push-token registration (no-op transport until F6)
- [x] **Explicit decision:** mobile is online-first with read cache in Phase 2; a full offline sync protocol is deferred (analysis §4.4 — no premature sync engine). Recorded in mobile/README.md so it isn't rediscovered as a gap
- [x] Jest/RNTL (9 suites / 12 tests) + Maestro smoke flow committed with documented run command (not executed locally — no emulator; noted in Log)

### Wave F5 — AI reflection (1 agent, after F2)

- [x] LLM port in product ports crate (product-owned per analysis §4.4 — AI is not a baukit crate); default adapter for the Anthropic API (claude-sonnet-5, key/model/endpoint via env config, stub-tested without live key), provider swappable behind the port
- [x] Opt-in per user (default OFF, `/me/ai-settings`); on-demand entry reflection + weekly summary via outbox → worker job (idempotency keys, SKIP LOCKED leasing, 3-attempt exponential backoff, terminal failure handling, spec §2.4 worker metrics incl. queue age; separate worker binary on baukit runtime/ops conventions)
- [x] Privacy documentation (backend/docs/ai-privacy.md incl. Anthropic 30-day retention + ZDR references): exactly which content leaves the system, to whom, retention; reflections stored as their own entity, never fed to analytics

### Wave F6 — reminders + streaks (1 agent, after F2)

- [ ] Streak computation service (entry-per-day with timezone + grace rules, property-tested), daily reminder scheduling in the worker with quiet hours
- [ ] Notification port with Expo push adapter (mobile); email port stubbed behind the same pattern (deliverability deferred per analysis §4.3)

### Wave F7 — telemetry + conformance (1 agent, after F3/F4)

- [ ] baukit-test conformance suites wired (ops, metrics spec §6, OpenAPI drift); domain metrics under `leitbild_` prefix; worker metrics on spec names
- [ ] Shared dashboard + burn-rate alerts from `deploy/observability` render against the app **unmodified** — the Phase 1 exit criterion re-proven on a generated app

### Wave F8 — deploy + Phase 2 exit criterion (sequential, 1 agent)

- [ ] Chart values + environments for leitbild (api, worker, migration job, Postgres via compose-equivalent or CNPG-lite, Keycloak) deployed to a disposable k3d/K3s node; smoke: login + CRUD + `/metrics` scraped
- [ ] **The timed run:** clean machine, stopwatch — `baukit new` → authenticated CRUD with telemetry deployed, target < 1 hour, zero manually copied files. Measured time + every friction point logged; misses are platform bugs to fix and re-run, not footnotes
- [ ] Node is disposable and torn down; durable hosting is Phase 3

**Focus F exit:** roadmap Phase 2 exit criterion met and measured; F5/F6 pillars may continue past it.

## Focus G: architecture health platform rewrite (Phase 2.5, after Focus F Wave F2)

The analysis' chosen dogfood target (§ executive summary). Per user (2026-08-08): full rewrite in that project — scaffolded fresh with `baukit new`, existing code is reference material only, no mechanical migration. Planned Focus-D-style: scout first, detailed waves authored from the scout output.

### Wave G0 — read-only scout (1 agent)

- [x] Inventory the existing architecture health platform repo: features, data model, integrations, analysis pipelines, deploy, tests; produce a keep/drop/defer list for the rewrite and a wave plan appended to this section (as Focus D's scout did)
- [x] Decide frontend targets for the rewrite (likely web-only) and which capabilities (`--auth`, worker) the scaffold needs

Scout summary (2026-08-09, full report: [architecture-health-platform-rewrite-scout.md](./architecture-health-platform-rewrite-scout.md)): existing repo is a 139k-line TS prototype (Next.js 16 + tRPC + Prisma [88 models] + BullMQ [31 queues], 236 tRPC procedures, 52 routes, no active code CI) with prototype-level cohesion — specs/fixtures/vocabulary are reference material only. Rewrite: `baukit new architecture-health-platform --backend --web --auth oidc` (NO --mobile), worker is mandatory but the generator has no `--worker` (logged as platform gap — product-local worker crate added after scaffold proves green). Keep core: OIDC + orgs/projects/repos, GitHub-first provider port, durable Postgres job/outbox worker, tree-sitter scanner/graph, high-confidence declarative rules, versioned honest health scoring, findings lifecycle by fingerprint, quality gates, one Rust/OpenAPI contract, web dashboard, local CLI + GitHub Action, PR delta later. Drop: tenant layer, billing/licensing, Clerk/SAML, executable custom rules/isolated-vm, eBPF agent, Bull Board, product Terraform, fake scoring inputs. Defer: other git providers, issue trackers, doc ingestion/vector search, runtime telemetry, LLM features, ADR governance, compliance, reports/notifications, benchmarks. 16 platform-gap candidates logged in the scout report §5 (headline: worker generation in CLI/templates, durable job/outbox as extraction candidate, queue readiness/age helpers).

### Wave G1 — scaffold + walking skeleton (sequential, 1 agent)

- [x] Fresh orphan `baukit-rewrite` branch from `baukit new architecture-health-platform --backend --web --auth oidc`, pinned to `baukit-v0.2.0`; old code stays as reference history only, no copied product files
- [x] Generated backend/web full local CI-equivalent green (Docker-gated tests, OpenAPI drift, frontend lint/typecheck/test/build, `baukit doctor` "product is healthy") **before product code**
- [x] Compose up PostgreSQL + Keycloak; PKCE login → generated protected `/me` → internal user bootstrap demonstrated (stable subject→UUID mapping verified in Postgres)
- [x] Record every scaffold/manual step in the Log, especially the missing worker generation capability (20 friction items logged)

### Wave G2 — core domain + persistence (1 agent)

- [x] Replace example domain with User/UserIdentity, Organization/OrganizationMember, Project/ProjectRepository, Repository/RepositoryConnection, AnalysisRun, persistent Finding, FindingOccurrence, FindingStatusHistory, Dependency, AnalysisMetric, HealthScore, RulesetConfig, QualityGateConfig, QualityGateEvaluation, JobOutbox (claimable scheduling/locking columns ready for G3)
- [x] Domain provider-free; ports for repositories/clock/ID gen/job dispatch; services own authz + use cases; SQLx adapters own PostgreSQL (transactional aggregate writes); 5 ordered migrations with composite org FKs, lifecycle constraints, history indexes — no JSON dumping of known lifecycle/query fields
- [x] Unit/property tests: membership scope, framed SHA-256 finding fingerprints with normalized paths, status transitions, auto resolve/reopen preserving manual resolutions, sortable IDs, health-history ordering; PostgreSQL integration tests via baukit-test (12 tests incl. 2 Docker)

### Wave G3 — repository intake + worker foundation (1 agent after G2; ∥ G4)

- [ ] Integrations + worker crates + worker bin; product-local durable Postgres job/outbox (`repository.sync`, `analysis.run`, `analysis.schedule`, `webhook.process`)
- [ ] Git provider port + GitHub adapter: credentials, repo catalog, verified raw-body webhook signatures, platform-ID→internal-repo resolution, idempotent delivery IDs, scoped temp clone credentials
- [ ] Manual/default-branch/scheduled scans; bounded retry/backoff, per-repo concurrency exclusion, cancellation/timeout, temp-dir cleanup, crash recovery
- [ ] Worker on baukit runtime/telemetry/ops with exact spec §2.4 worker metric families incl. queue age; integration tests via local bare Git fixture + mocked GitHub HTTP/webhooks

### Wave G4 — scanner, parsers and dependency graph (1 agent after G2; ∥ G3)

- [x] Analysis crate: ignore-aware scanner (gitignore + built-ins, generated-file detection, structured skip records), file-size/line limits, deterministic path normalization, explicit language capability matrix (6 languages; TS/JS + Rust first-class)
- [x] Tree-sitter parsers + import/entity/function extractors for TS/JS and Rust first, then Python/Java/Go behind isolated language modules with golden fixture repos; graceful syntax-error diagnostics
- [x] Directed dependency graph, SCC/cycle canonicalization, module metrics (fan-in/out, size, cycle membership), source locations; analyzed repo code is never executed
- [x] Property tests for graph invariants/cycle stability (3 proptests × 768 generated cases); golden tests for monorepos, aliases, relative imports, generated files, parse failures, oversized files

### Wave G5 — rules, scoring and analysis result contract (1 agent after G4)

- [ ] Versioned declarative rule catalog (initial high-confidence structural/code-quality/security/reliability rules); rule IDs + finding fingerprints are stable public contracts
- [ ] Per-repo ruleset resolution: enable/disable, severity, typed threshold overrides; no executable custom code, no marketplace
- [ ] Versioned health scoring from measured components only, with evidence/confidence; missing components redistribute weight explicitly, never guessed values
- [ ] Analysis result: scan summary, dependencies, findings, metrics, component/overall scores, topology; deterministic JSON snapshots + regression fixtures for score/rule changes

### Wave G6 — analysis orchestration + REST/OpenAPI contract (1 agent after G3/G5)

- [ ] Worker transaction flow: claim → clone → analyze → persist → reconcile findings → commit outbox → clean workspace; idempotent retry cannot duplicate a completed run
- [ ] Authenticated endpoints: /me, organizations/projects CRUD, repositories CRUD, analyses trigger/list/get, repo health/history/findings/topology/ruleset, finding status update, GitHub webhook
- [ ] Cursor pagination, org-scoped authz everywhere, standard envelope, request IDs, documented conflict/rate/validation responses
- [ ] Deterministic committed OpenAPI + generated TS client; API/worker/Postgres integration test proves trigger → job → completed analysis → query results

### Wave G7 — web product (1 agent after G6; ∥ G8)

- [ ] Vite/TanStack authenticated shell with org/project selection and repository onboarding
- [ ] Repository dashboard: analysis state, versioned overall/component score, trend, finding counts, top findings, manual re-run
- [ ] Findings table/detail/status workflow, health history, dependency/topology graph, per-repo ruleset editor; accessible loading/error/empty states, zero placeholder data
- [ ] @baukit/ui-tokens, generated API client, privacy-safe structural analytics; Vitest/Testing Library + Playwright login → connect fixture repo → analyze → inspect finding/topology → change finding status

### Wave G8 — local CLI + CI integration (1 agent after G5; ∥ G7)

- [ ] Rust CLI `scan`/`analyze`/`gates` running the same analysis crate locally; JSON + human output, exclude/ruleset config, score threshold, stable exit codes
- [ ] GitHub Action wrapper: install/run CLI, safe caches, summary/annotations, pass/fail status, no source upload
- [ ] Quality-gate templates (strict, relaxed, monolith, modular-monolith, microservices) re-authored against new rule IDs
- [ ] CLI golden output + exit-code tests; Action fixture workflow covers pass/fail, malformed config, below-threshold

### Wave G9 — remediation + PR governance (1 agent after G6/G8)

- [ ] Persistent finding lifecycle reconciles occurrences by fingerprint, preserves ack/assignment/status history, auto resolve/reopen only under tested rules
- [ ] GitHub PR job analyzes real head/base, derives changed files/lines from Git, computes finding delta/risk from actual graph data, posts bounded review comments
- [ ] Quality-gate evaluation persisted per analysis with immutable config snapshot; PR status and CLI evaluate the same result; false-positive annotation never mutates history
- [ ] E2E fixture: webhook replay, PR synchronize, new/resolved/worsened findings, comment dedup, blocking/non-blocking gates

### Wave G10 — telemetry + conformance gate (sequential, 1 agent after G7–G9)

- [ ] baukit-test ops/auth/metrics/OpenAPI conformance for API and worker; gating readiness checks Postgres and job store
- [ ] Standard HTTP + worker metric families exactly once; product metrics `architecture_health_platform_` prefix with bounded labels only
- [ ] Logs/traces scrub source, diffs, repo credentials, webhook bodies, tokens; workspace paths/repo names never metric labels
- [ ] Full local CI-equivalent gate green: fmt/clippy, unit/property/integration incl. Docker, coverage, deny/MSRV, OpenAPI/client drift, frontend lint/typecheck/coverage/build, Playwright, CLI/Action fixtures

### Wave G11 — deploy + rewrite exit gate (sequential, 1 agent)

- [ ] Separate API/worker/migration images; worker image has Git + parser assets + bounded scratch volume, API image does not
- [ ] Shared baukit chart values: API + worker + release migration, private ops listeners, Postgres/Keycloak, worker egress to GitHub, default-deny netpol; no Redis/Qdrant/MinIO
- [ ] Deploy to disposable k3d/K3s; smoke: OIDC login → connect fixture repo → analysis completes → dashboard renders → /metrics scraped → PR/CLI gate agrees
- [ ] Shared dashboard + burn-rate/worker alerts render unmodified; graceful shutdown proves API drain + in-flight job recovery
- [ ] Restore/deploy rehearsal + documented rollback; baukit rewrite becomes the release branch

### Extraction review gate (orchestrator, after F and G both run)

- [ ] Review product-local code repeated across leitbild and the rewrite (web/mobile OIDC client code, LLM port, notification port, outbox) against the two-consumer guardrail (analysis §16): promote proven seams into baukit / `@baukit/*` or record them as deliberately product-owned in analysis §17

## Log

- 2026-08-08: Task list created; Wave 1 dispatched to Codex.
- 2026-08-08: Phase 2 plan authored (Focus E–G) — platform gaps (identity port `baukit-auth`, auth templates + Keycloak compose, friction backlog, `baukit-v0.2.0`), journaling reference app `leitbild` (backend+web+mobile, guided authoring program / entries / AI reflection / reminders, per user decisions), architecture health platform full rewrite as scout-first Focus G. Nothing implemented yet.
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
- 2026-08-09: G0 complete (codex, read-only) — architecture-health-platform scouted; full report saved to docs/architecture-health-platform-rewrite-scout.md; wave plan G1–G11 authored into Focus G; 16 platform-gap candidates logged (headline: no --worker in generator). Static quality findings confirm rewrite-not-migrate (missing worker deps, phantom Prisma fields, hard-coded scoring inputs, dead webhook handler, Helm probing nonexistent route).
- 2026-08-09: User decision — GitHub Actions free minutes exhausted; user will not pay for hosted runners (may self-host later, e.g. Blacksmith). Until then every "CI green" gate in Focus F/G is satisfied by running the full CI-equivalent locally (same commands, Docker-gated tests included); pushes proceed without waiting on Actions runs.
- 2026-08-09: E4 complete (orchestrator + codex prep) — full local gate green (make ci, workspace tests --include-ignored, observability lint, auth fixture backend+web+mobile), release-train.sh minor bump to 0.2.0 (Changesets tried to escalate to 1.0.0 over internal peer deps, normalized back; CLI snapshots version-sensitive, updated), coherence green (9 crates, 6 packages), commit 41fcf52 pushed, tag baukit-v0.2.0 pushed. **BLOCKER (user action):** GitHub Actions refuses ALL jobs on the account (every job fails in ~2s with zero steps/logs, rerun identical, Actions enabled, workflow valid — billing/spending-limit signature). CI runs 31280863546 attempts 1+2 failed this way. User notified; rerun CI on main once billing is fixed. Local verification mirrored the exact CI command set.
- 2026-08-09: G4 complete (codex, commits 01494c2/f46fe28 on architecture-health-platform `baukit-rewrite`) — pure `architecture-health-platform-analysis` crate (no DB/migrations/worker/API changes): deterministic gitignore-aware scanner with built-in ignores, generated-file detection, size/line limits, normalized paths, structured skip records; tree-sitter import/entity/function extraction for TS/JS/Rust (first-class) + Python/Java/Go (isolated modules) with graceful syntax-error diagnostics; static dependency resolution (relative imports, aliases, monorepos, language module conventions), repo code never executed; directed graph with SCC detection, canonical cycles, source locations, fan-in/out/size/cycle-membership metrics; committed fixture repos. Gate green: 13 analysis-crate tests (3 scanner, 6 golden, 4 graph incl. 3 proptests × 768 cases), full workspace `--include-ignored` 25 passed incl. 2 Docker, fmt/clippy/openapi_drift+conformance/make check; built + rebased + pushed from isolated clone (G3 active in shared checkout). Friction: (1) Jinja `Cargo.toml` parse noise (recurring); (2) `make check` omits Docker-mandatory ignored tests (recurring); (3) drift/conformance need separate explicit invocations to document independently; (4) no baukit primitive for static scanning/capability matrices/source locations/dependency graphs — all hand-built (likely fine: product domain, but note for extraction review); (5) concurrent workspace-member + lockfile work forced isolated clone + manual pre-push rebase (orchestration note); (6) isolated full-workspace build exhausted disk — disposable target cleaned, rerun against shared Cargo target cache; (7) fixture paths like `node_modules` needed git force-adds.
- 2026-08-09: F5 complete (codex, commits 6b2df33/eaf6292/ce20f4d on leitbild main) — AI reflection: product-owned `Llm` port + swappable Anthropic adapter (default claude-sonnet-5, `LEITBILD__AI__{API_KEY,MODEL,ENDPOINT}`, stubbed HTTP contract test, no live key needed); per-user opt-in default OFF; durable Postgres outbox (idempotency keys, SKIP LOCKED leasing, stale-lease reclaim, 3 attempts w/ exponential backoff, terminal failure state); separate worker binary on baukit runtime/telemetry/ops with readiness + graceful shutdown; entry reflections + Mon–Sun weekly summaries stored as own entity (journal content never overwritten); worker metrics `worker_job_runs_total`/`worker_job_duration_seconds`/`worker_queue_oldest_age_seconds`; migration 0004; privacy contract backend/docs/ai-privacy.md (Anthropic 30-day retention + ZDR cited); routes: GET/PUT /me/ai-settings, POST /journal-entries/{id}/reflection, POST /reflections/weekly, GET /reflections[/{id}]; OpenAPI + TS client regenerated; worker enabled in deploy/values.yaml. Gate green: fmt/clippy/`--include-ignored` 26 passed incl. all 5 Docker/openapi_drift 2/make check; pushed via isolated clone rebased onto concurrent web commits. Friction: (1) no worker generator — crate/bin/config/lifecycle/ops/readiness/deploy/Make target all manual (headline, matches G-scout gap); (2) no baukit durable job/outbox facility — schema, SKIP LOCKED claiming, leases, retries, atomic completion all product-local (2nd product to hand-roll this; extraction candidate); (3) telemetry v0.2.0 has histogram buckets only for HTTP duration — worker duration exports as summary; product-local fixed-bucket shim needed to satisfy §2.4 conformance; (4) metrics conformance helper doesn't require the §2.4 queue-age family — explicit product test needed; (5) `make check` excludes ignored Docker tests (recurring); (6) CLI not on PATH (recurring); (7) Jinja `Cargo.toml` parse noise (recurring); (8) `PostgresItemRepository` demo naming keeps absorbing unrelated adapters (recurring, F1 #2).
- 2026-08-09: G2 complete (codex, commits c1099ac/148cebf on architecture-health-platform `baukit-rewrite`) — core domain + persistence: provider-neutral entities (users/identities, orgs/memberships, projects/repos/connections, analysis results, findings + lifecycle history, config, quality gates, durable jobs); stable framed SHA-256 finding fingerprints with normalized paths; auto resolve/reopen preserving manual resolutions/dismissals; versioned health scores from measured components with evidence only; ports for repositories/clock/sortable IDs/authz/job dispatch; org-scoped authz in services; SQLx Postgres adapter with transactional aggregate writes; `/me` bootstrap migrated to User/UserIdentity; 5 ordered migrations (identity/orgs, projects/repos, analysis results, finding lifecycle, config + claimable outbox). Gate green: fmt/clippy `-D warnings`/`--include-ignored` 12/12 (7 domain property/unit, 3 conformance, 2 Docker Postgres)/make check/openapi_drift; web untouched. Friction: (1) Jinja `Cargo.toml` parse noise (recurring); (2) generated Makefile `check` skips ignored Docker tests — needed separate `--include-ignored` run (recurring, G1 #15); (3) baukit has no durable job/outbox abstraction or migration template — claim/retry/idempotency/locking schema hand-designed (extraction candidate, matches scout §5); (4) generated OIDC bootstrap models only `user_id + subject` — issuer-scoped User/UserIdentity split required manual API/service/in-memory/Postgres rewiring; (5) removing example routes leaves committed OpenAPI regeneration a separate manual command — check only reports drift, no regen helper.
- 2026-08-09: F3 complete (codex, commits 38445aa/a2a593f on leitbild main) — web app: typed API via generated/openapi.d.ts + baukit `createApiClient`; OIDC authorization-code + PKCE with refresh and Strict-Mode-safe callback dedup; journal CRUD (markdown editor, date/tags/mood, pagination, text search, comma-separated tag filter); program catalog, deterministic resume, stage/section navigation, debounced autosave with revision-conflict resolution, progress + word targets, completion review with markdown answer export; ui-tokens responsive theme; privacy-allowlisted analytics on no-op transport, content-safe error handling; one-command real-stack e2e (`corepack pnpm test:e2e`). Gate green: frozen install, build/lint/format, vitest 8/8, Playwright real-stack critical path 1/1 (containers + private failure artifacts removed), backend re-verified fmt/clippy/`--include-ignored` 22/22/openapi_drift 2/2/make check; mobile/ untouched. Friction: (1) Jinja `Cargo.toml` parse noise (recurring); (2) generated OIDC scaffold requests `offline_access` but the seeded public Keycloak client rejects offline tokens — scope corrected manually (mirror of G1 #17, generator seed inconsistency); (3) React Strict Mode double-fires the OIDC callback effect — needed callback-promise dedup to protect one-time PKCE transaction (template gap); (4) backend defaults to no browser CORS origins while scaffold points Vite at :8080 directly — manual `/api` Vite proxy required; (5) Playwright emits ARIA error-context artifacts even with traces/screenshots/video off — harness must scrub artifacts to hold the content-privacy boundary; (6) no Keycloak readiness contract in compose for test harnesses (recurring); (7) generated TS \"client\" is type declarations only, not endpoint functions — products still hand-write typed wrappers over `createApiClient<paths>`; (8) no complete web format / real-stack orchestration command in scaffold — browser install, migrations, seeding, startup, teardown wired manually.
- 2026-08-09: F4 complete (codex, commits acc875e/5c13019 on leitbild main) — Expo Router mobile app: OIDC PKCE auth; journal CRUD (date grouping, markdown, tags, mood, pagination/search, comma-separated tag queries); guided-program catalog + run lifecycle with serialized autosave revisions, conflict handling, stage/run completion; online-first owner-scoped Expo SQLite read cache implementing `@baukit/data-contracts` (product-local adapter); generated OpenAPI types consumed via openapi-fetch; no-op push-registration transport pending F6; privacy rule enforced (content cannot enter telemetry/logs/error objects); online-first + deferred-sync decision documented in mobile/README.md; Maestro login/create-entry flow committed (.maestro/login-create-entry.yaml) with documented command but NOT executed — no Maestro/emulator available locally. Gate green: tsc, eslint 0 warnings, Jest/RNTL 9 suites / 12 tests; backend re-verified fmt/clippy/`--include-ignored` 22 tests/openapi_drift. Friction: (1) Expo installer invoked a different pnpm store than corepack 11.18.0 — deps installed directly with corepack; (2) `@baukit/data-contracts` needed a manual `allowBuilds` entry (recurring theme); (3) data-contracts ships contracts + in-memory impl but no Expo SQLite adapter — product-local adapter required (extraction candidate); (4) baukit ESM import-only exports unresolvable by Jest 29 without manual moduleNameMapper/transform config; (5) optional Vitest peer of data-contracts installs even under Jest; (6) `unrs-resolver` needed another allowBuilds entry; (7) Expo suggested nonexistent `jest-expo` 57.0.8 — pinned 57.0.3 manually; (8) Jinja `Cargo.toml` parse noise (recurring); (9) push-registration endpoint/provider doesn't exist until F6 — no-op seam required; (10) Maestro not runnable headlessly here — flow committed + documented only; (11) shared dirty worktree (concurrent web/ agent) blocked pull/rebase — push done from isolated clean clone, local ref reconciled after (parallel-agents-in-one-repo friction, orchestration note).
- 2026-08-09: G1 complete (codex, commits c44ccd1→419a488 on architecture-health-platform `baukit-rewrite` orphan branch) — fresh scaffold via release CLI binary (`--backend --web --auth oidc`, no mobile), zero code copied from main, baukit sources untouched; pristine scaffold commit first, then separate fix commits (SSH+`baukit-v0.2.0` pinning, `.cargo/config.toml` git-fetch-with-cli, Cargo+pnpm 11.18.0 lockfiles, `allowBuilds` policy, docs corrections, full CI coverage, rustfmt fix, seeded-user `offline_access` role). Full local gate green before any product code: fmt/clippy `-D warnings`/`--include-ignored` (auth+ops conformance, OpenAPI drift, Docker Postgres test), web frozen install/build/eslint/vitest, `baukit doctor` healthy. Walking skeleton: Postgres 17 + Keycloak 26.7, 2 migrations, `/me` 401→PKCE S256 login (azp/aud/scope verified)→200 with stable internal UUID, healthz/readyz/metrics green, clean teardown. Friction (20 items; ★ = new vs F0): (1) ★headline: generator has no `--worker` flag — rewrite needs product-local worker crate; (2) CLI not on PATH (recurring); (3) ★`baukit new` can't target an existing orphan repo root — temp dir + manual move; (4) no git init/remote/commits (recurring); (5) HTTPS + wrong tag `v0.2.0` deps (recurring); (6) missing `.cargo/config.toml` (recurring); (7) no lockfiles (recurring); (8) pnpm 11 `allowBuilds` missing (recurring); (9) git-hosted TS packages run nested prepare builds (recurring); (10) docs wrong: "Backend-only", wrong URL/tag, duplicated setup text, ★describes nonexistent mobile support; (11) ★no-mobile scaffold still emits a mobile Keycloak client; (12) ★generated Rust sources not rustfmt-clean; (13) generated CI backend-only, no `--include-ignored`, no deploy-key setup (recurring); (14) Actions billing-refused (known); (15) ★generated Makefile `check` omits ignored Docker tests and web gates; (16) Keycloak compose has no healthcheck (recurring); (17) ★seeded user lacks `offline_access` role while web client requests it — token exchange 400 until realm seed fixed; (18) no one-command walking-skeleton smoke helper (recurring); (19) Jinja `Cargo.toml` parse noise (recurring); (20) `.gitignore` ignores `/generated/` that `baukit.toml` targets (recurring, also F2 #4).
- 2026-08-09: F2 complete (codex, commits 2f9909c/485b5c4/364f5ad on leitbild main) — API + contract: authenticated routes for journal entries CRUD (`GET|POST /journal-entries`, `GET|PUT|DELETE /journal-entries/{id}` with pagination/search), program catalog (`GET /programs`, `GET /programs/{id}`), program-run lifecycle (`GET|POST /program-runs`, `GET /program-runs/{id}`, `/resume`, `PUT .../sections/{id}/response`, `POST .../stages/{id}/complete`, `POST .../complete`), and `/me` profile bootstrap. Standard envelope everywhere; foreign entries/runs return owner-scoped 404; autosave conflicts return `revision_conflict` with expected/current revisions; privacy rule held (no content in logs/traces/metrics/analytics). OpenAPI committed (`backend/openapi.json`) with extended drift/route assertions; TS client regenerated via `baukit generate openapi-client` into `generated/openapi.d.ts` — regenerated twice with identical hashes. Gate green: fmt/clippy `-D warnings`; `cargo test` 18 passed; `--include-ignored` 22 passed incl. all 4 Docker/Postgres tests; openapi_drift 2 passed; make check; web+mobile tsc. Deps still SSH-pinned to baukit-v0.2.0; baukit untouched. Friction: (1) cargo metadata scans raw Jinja `Cargo.toml` templates in the baukit checkout, printing non-fatal parse errors during clippy/tests/codegen (recurring, also F0 #14 / F1 #1); (2) `ApiQuery<Vec<String>>` doesn't accept the expected repeated-tag query encoding and only yields a generic validation envelope — contract had to document tags as comma-separated; (3) baukit has extractor rejection envelopes but no field-level DTO validation helper — bounds/semantic field maps hand-rolled via `ApiError::validation`; (4) scaffold's `.gitignore` ignores `/generated/` even though `baukit.toml` points the generated client there — ignore entry had to be removed to commit the required artifact.
- 2026-08-09: F1 complete (codex, commits 9f00487/d9f472a/7aecb3b on leitbild main) — domain + persistence: journal entries (date/markdown/tags/mood, user-owned), content-free daily entry facts for F6 streaks, versioned programs (ordered stages/sections, prompts/guidance/word targets), program runs (draft autosave with optimistic revisions, deterministic resume, ordered stage completion, final completion); migration 0003 adds 8 tables, release-job-only convention kept; original guided program "Deliberate Direction v1" authored as versioned JSON data (4 stages / 16 sections), seed binary idempotent (double-run verified: v1 inserted, second run no-op). Gate green: fmt/clippy `-D warnings`/tests `--include-ignored` (17 fns incl. 3 Docker Postgres; 4 proptests × 32 cases for ordering/resume/revision/completion invariants)/openapi_drift/make check; deps still SSH-pinned to baukit-v0.2.0; privacy rule enforced (no content in logs/metrics). Friction: (1) cargo prints non-fatal `error:` noise scanning unrendered Jinja `Cargo.toml` templates in the baukit checkout — exclude from discovery or rename; (2) generated `PostgresItemRepository` naming doesn't scale past the demo item — neutral or per-aggregate adapter convention needed; (3) no template convention for versioned data seeding / release sequencing alongside migrate+openapi binaries; (4) manually boxed `PortFuture` port traits create adapter/mock boilerplate — endorse native async-trait convention.
- 2026-08-09: F0 complete (codex) — leitbild scaffolded via `baukit new leitbild --backend --web --mobile --auth oidc`, private repo `PatrickKoss/leitbild` created (SSH remote), baukit consumed via git deps pinned to tag `baukit-v0.2.0` (resolves to 41fcf52), 4 commits (pristine scaffold first, fixes separate). Full local CI-equivalent green: backend fmt/clippy `-D warnings`/tests incl. Docker-gated + auth/ops conformance + OpenAPI drift; web + mobile pnpm 11.18.0 frozen install/build/tsc/lint/vitest. Walking skeleton demonstrated: compose up Postgres 17 + Keycloak 26.7, migrations applied, `/me` 401 without token → 200 via seeded-user PKCE authorization-code login, healthz/readyz/metrics green, clean teardown. **Scaffold friction (each a platform bug candidate):** (1) built CLI not installed on PATH; (2) generator does not git-init or set remote; (3) generated baukit deps used HTTPS + wrong tag `v0.2.0` instead of SSH + `baukit-v0.2.0`; (4) no `.cargo/config.toml` with `git-fetch-with-cli` for private SSH fetch; (5) generated docs called the combined product "Backend-only", wrong tag/URL, recommended `--baukit-path` against the release-consumption contract; (6) no Cargo.lock/pnpm lockfiles generated → first run not reproducible; (7) pnpm 11 rejects baukit git-package prepare scripts without a generated `allowBuilds` policy; (8) plain `pnpm` resolved to global 10.15.1 despite manifest pin — needed explicit `corepack pnpm@11.18.0`; (9) pnpm-10 stale lockfile tripped pnpm 11 release-age policy; (10) git-hosted TS packages run nested npm prepare builds (slow, mixed package managers); (11) generated CI has only a backend job — web/mobile jobs and `--include-ignored` missing; (12) generated CI lacks SSH/deploy-key setup for private baukit (manual deploy key + `BAUKIT_DEPLOY_KEY` secret); (13) Actions still refused account-wide (billing, known); (14) cargo emits non-fatal parse errors for raw Jinja `Cargo.toml` files in the tagged baukit checkout; (15) Keycloak compose service has no healthcheck — manual OIDC discovery poll; (16) no one-command walking-skeleton smoke helper; (17) root README repeats the compose paragraph and omits the automated PKCE smoke procedure.
- 2026-08-08: E2 complete (codex) — `baukit new --auth oidc` (manifest `[capabilities] auth = "oidc"`, omitted when absent); backend auth flavor: protected GET /me with bearer OpenAPI security, issuer/audience config, subject→internal-user mapping service+repo+migration, mock-OIDC conformance tests; Keycloak realm.json (test/password user, confidential backend client, public PKCE web/mobile clients, explicit backend audience mapper — added after live smoke exposed missing aud claim); product-local web PKCE auth.ts + Expo AuthSession/SecureStore equivalent; auth flavor in CI fixture matrix; golden snapshots updated. Verified: CLI tests, non-auth combined fixture + auth fixture both fully green (Docker-gated included), live Keycloak smoke (401 no token / 200 with token). Friction: corepack picked pnpm 11.20.0 over pinned 11.18.0 until activated.
- 2026-08-08: E3a complete (codex) — baukit-test MetricsConformanceOptions with opt-in worker-family enforcement (worker_job_runs_total{job,outcome} outcomes success|failure|retry, worker_job_duration_seconds histogram family, exact label sets; default mode unchanged); reqwest single-version confirmed (0.12.28 via workspace pin). baukit-config env values keep source strings (secrets '0123'/'1e5' survive literally; standard vs product config deserialized separately), thiserror pin relaxed to caret. fmt/clippy/tests --include-ignored (28/28 incl. Docker)/deny/MSRV green.
- 2026-08-08: E3b complete (codex) — baukit-http gains with_additional_allowed_headers (extend-not-replace CORS) + with_json_rejection_code (default stays validation_failed); baukit-telemetry OTEL_SDK_DISABLED now truly skips tracer/OTLP/span processor/background tasks (spec-conform bool parsing, subprocess test) with sdk_disabled builder + is_otel_sdk_disabled; baukit-ops adds instrumented begin(&PgPool) + crate-level guidance (raw &PgPool executors stay invisible — SQLx has no acquisition hook). fmt/clippy/tests/MSRV green.
- 2026-08-08: E1 complete (codex) — baukit-auth crate: IdentityVerifier port + Principal (subject/org/tenant only), OidcVerifier (eager discovery, lazy cached JWKS with rotation-on-unknown-kid, fail-closed, RS256 default + RSA/ECDSA/Ed25519 allowlist), Keycloak-shaped issuer convention via standard OIDC metadata only, Axum extractor with 401 unauthenticated/403 permission_denied envelope + WWW-Authenticate, bearer scheme via existing baukit-openapi opt-in. baukit-test: mock OIDC/JWKS server, RS256 minting, rotation/delay fixtures, check_auth_router_conformance (expired/wrong-aud/wrong-iss/unsigned). jsonwebtoken crypto-provider conflict removed at origin (ring directly). Full gate green: fmt/clippy all-features/test --include-ignored (Docker)/MSRV 1.95/deny/version coherence.
- 2026-08-08: E0 complete (codex) — all four perishable claims re-verified with zero material deltas (TanStack Start still RC, Loki SSD deprecated/removal in 4.0, Expo SDK 57 == template pin, PostHog hobby baseline unchanged); decisions 15–18 appended to analysis §17, no template changes. Follow-ups noted: re-check Loki before 4.x, reassess PostHog Cloud vs self-host before production.
- 2026-08-08: D2a-fix complete (codex, commit a651da2) — worker metrics on spec §2.4 names/labels (`worker_job_runs_total{job,outcome}`), domain metrics untouched; fmt/clippy/tests green.
- 2026-08-08: D2b complete (codex, commit d87d4ef) — /readyz healthchecks on ops ports 19464/19465, BAUKIT_DEPLOY_KEY SSH auth across backend CI + frontend E2E, vendored tgz in E2E image, engine-strict=false for Node 20, deterministic gen-client, docs. Deferred to D3: authenticated Docker build + E2E (no SSH agent in agent shell), repo-wide lint, in-place gen-client.
- 2026-08-08: D2a complete (codex, commit 19bbe01) — domain metrics conformed and described, worker job metrics instrumented, readiness acquisition via baukit_ops::acquire; fmt/clippy/tests green. Codex correctly flagged a spec conflict: orchestrator's prompt contradicted telemetry-spec §2.4 on worker metric names → D2a-fix dispatched. Gap: baukit_ops::acquire can't instrument implicit PgPool executors/Pool::begin (7 sites listed in transcript).
- 2026-08-08: D2c complete (codex, commit 4336f32) — TS schema regenerated, legacy envelope parsing removed, envelope coverage added; typecheck/coverage (86.54%)/build green, repo-wide lint deferred to D3. Flagged: openapi.json emits 147 query params as path params → D3 item.
- 2026-08-08: D1b complete (codex, commit 9bc0590) — baukit-http layer stack + envelope (product codes preserved, SCIM keeps RFC 7644 shape), 404/405/extractor normalization, exact-once spec §2.1 metrics, deterministic openapi.json + drift test; fmt/clippy/tests green. Friction: fixed CORS header set needs product extension (Accept, x-webhook-secret); JSON extractor code `validation_failed` vs product `invalid_json`; template Cargo.toml diagnostics again.
- 2026-08-08: D1c complete (codex, commit 7abe09e) — @baukit/api-runtime vendored + adopted in api-client and Expo app, dual-envelope error tolerance, 401 refresh preserved; lint/typecheck/coverage (86.54%)/build green. Friction: baukit TS packages declare Node >=24 while product CI runs Node 20 (EBADENGINE); repo does per-scope npm installs, not one workspace install.
- 2026-08-08: D1a complete (codex, commit c16cefc) — baukit runtime/telemetry/ops adopted for api/worker/migrate/seed, staggered drain, gating Postgres readiness + pool metrics, /buildinfo; fmt/clippy/tests green. Friction: no complete OTEL_SDK_DISABLED switch (mapped to zero sampling); acquisition timing only via baukit's helper.
- 2026-08-08: D0 complete (codex, commit 59b3614) — BaukitConfig<ProductConfig> bridge with legacy env aliases, Secret<String>, shared loader across api/worker/migrate/seed; fmt/clippy/tests green. Friction noted for platform backlog: env parsing coerces numeric-looking secrets; private git deps needed `git-fetch-with-cli`; unrendered template Cargo.tomls emit parse diagnostics; exact `thiserror =2.0.20` pin advances consumer lockfiles.
