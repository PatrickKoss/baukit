# Code quality and security plan

Owner: Claude (orchestrator). Implementers: Opus 5.5 subagents at low effort, at most 10 in parallel (raised from 4 on 2026-11-07).
The orchestrator re-checks every subagent result (re-runs its commands, reads the diff, looks for skipped
checks and new suppressions) before ticking a box. Weak results go back to the same agent.

Status markers: `[ ]` open, `[~]` in progress, `[x]` done and verified, `[-]` dropped (reason in the log).

Scope: baukit and the products that consume it:
eigenruhe, hebkit, leitbild, redemut, schlauzug, tiefgang, solo-leveling-system,
lesezeichen, schluesselbund, kassensturz, runtime-analyzer, architecture-health-platform.

Raw exploration reports, with commands and `path:line` evidence: `docs/quality-security/E1..E4-*.md`.

## Principles

1. An existing linter beats a custom check. Custom checks go into the `baukit` CLI (`baukit check`), so
   every product runs the same code at the version it pins.
2. A new rule lands together with the fixes for its violations. No rule ships as "warn and ignore".
3. Suppressions need a reason: Rust uses `#[expect(lint, reason = "...")]`, TS uses
   `// eslint-disable-next-line rule -- reason`. Both get counted, and the counts may only go down.
4. Size limits get an allowlist of existing offenders that may only shrink. New code meets the limit on day one.
5. Products inherit rules from baukit. They can add rules but cannot remove them (`baukit doctor` checks this).

## Phase 0. Exploration

- [x] E1 baukit Rust crates, CLI and Rust templates: smells and lint gaps
- [x] E2 baukit TypeScript packages and web/mobile templates: smells and lint gaps
- [x] E3 product repos: recurring smells and drift from the baukit conventions
- [x] E4 security posture across baukit and the products
- [x] Orchestrator verification of E1 to E4 findings (see log)
- [x] Findings catalog and implementation phases

## Findings catalog

Verified findings only. IDs point into the reports.

### Code quality

| ID | Finding | Evidence | Prevention |
|---|---|---|---|
| Q-01 | Lint baseline lacks panic, indexing, cast, size and suppression rules | E1 totals: 42 `expect`, 42 indexing, 15 lossy casts, 11 bare `#[allow]` in baukit prod code; AHP 282 and RA 207 non-test `expect` (E3-03) | clippy lints (E1 "Recommended lint set") + shared `clippy.toml` |
| Q-02 | Three copies of the Rust lint table, products drifted | redemut has only `clippy::all`; SLS has only `all = "deny"`; RA `agent/` has none; hebkit is on edition 2021 | `baukit check lints`: product table must contain the baseline |
| Q-03 | God modules and functions | `cli/src/lib.rs` 3572 lines; `redemut-api/src/lib.rs` 8491; `schlauzug-api/src/lib.rs` 7420; AHP `limits.rs` 4369; template `api.rs::run` 205 lines, product copies 229 to 666 | `clippy::too_many_lines` (threshold 80) + `baukit check file-size` with shrinking allowlist; ESLint `max-lines-per-function`, `max-lines` |
| Q-04 | React hooks in baukit packages not linted | no `eslint-plugin-react-hooks` in `typescript/`; 5 violations in navigation and purchases-core (E2-02) | add the plugin to the shared config |
| Q-05 | Weak frontend lint configs in products | hebkit/mobile and SLS root use eslint-config-expo without typescript-eslint; hebkit has 152 `!.` and 77 non-test `as unknown as` (E3-04) | new `@baukit/eslint-config` package used by `typescript/`, templates and every product |
| Q-06 | Validators that end in casts, duplicated guards | 17 `as unknown as` in baukit src, `isRecord` copied in 10 packages (E2-03/04); 225 non-test `as unknown as` across products | one shared guard module; `consistent-type-assertions` with `objectLiteralTypeAssertions: never` + an ast-grep rule for `as unknown as` outside tests |
| Q-07 | Template-owned scripts forked in every product | 12 distinct `quality-gate.sh` hashes, 12 `openapi.sh`, 11 `reconcile_keycloak.py` (E3-05) | `baukit check template-files` against a hash manifest, with product extension hooks instead of edits |
| Q-08 | Copy-pasted wiring | `SuiteComposition` wiring copied into 8 products (E3-06) | builder in `baukit-suite`, template uses it |
| Q-09 | Agent instructions drift and contradict each other | AHP symlink rule vs hebkit `check-agent-instructions.sh:4`; eigenruhe CLAUDE.md:200 vs AGENTS.md:200; RA AGENTS.md is stale template text; 5 repos never mention `--include-ignored` | baukit-managed instruction block between markers, checked by `baukit check agent-docs` |
| Q-10 | No linter for scripts | baukit: 33 `.py`, 15 `.sh`, no ruff or shellcheck; products the same (only leitbild runs shellcheck) | ruff + shellcheck in baukit gate and template gate |
| Q-11 | cargo-deny half on | `bans`/`sources` configured but never run, fail today (12 wildcard errors); no deny for `cli/`; 9 products have no `deny.toml` | full `cargo deny check` everywhere, `allow-wildcard-paths = true` |
| Q-12 | Timing-based tests | 30 `sleep(` in baukit crates, 20 in tests (E1-13); products similar (E3-10) | `disallowed-methods` for sleeps, allowed with `#[expect]` only where a hang is simulated |
| Q-13 | Five product observability linters reimplement one idea | eigenruhe, kassensturz, AHP, leitbild, schluesselbund (E3 §6) | one `baukit check observability` wrapping `check-metric-names.py` |

### Security

| ID | Finding | Evidence | Prevention |
|---|---|---|---|
| S-01 | Access and refresh tokens in localStorage, no CSP | `auth-web/src/index.ts:428` | design change (decision D2) + test that no storage write holds a refresh token |
| S-02 | No security headers anywhere | nothing in `baukit-http`, templates or chart sets CSP, nosniff, frame-ancestors, HSTS | default `SecurityHeadersLayer` in baukit-http, CSP at the web edge, header test in the fixture gate |
| S-03 | Actions pinned by tag | baukit 0 of 80 SHA pins; release jobs hold `contents: write` + `id-token: write`; AHP fixture workflow has no `permissions:` | SHA pins via renovate `helpers:pinGitHubActionDigests`; zizmor + actionlint in every gate |
| S-04 | No secret scanning, no JS advisory gate | no gitleaks, osv-scanner, pnpm audit in baukit or 11 products (kassensturz has `security-audit.py`) | gitleaks + osv-scanner step in baukit gate and template gate |
| S-05 | Secret-bearing types derive Debug | `baukit-auth/src/api_token.rs:79` `IssuedApiToken` | redacting Debug or `secrecy::SecretString`; ast-grep rule on `derive(Debug)` + secret-named fields |
| S-06 | Clerk profile accepts tokens without `azp` | `baukit-auth/src/verifier.rs:1033` | make missing-azp acceptance opt-in; negative tests per profile |
| S-07 | Raw reqwest clients bypass baukit-egress | 10 call sites in eigenruhe, tiefgang, redemut, kassensturz, SLS (E4-08) | clippy `disallowed-methods` on `reqwest::Client::new/builder` outside an allowlisted module; migrate to `GuardedClient` |
| S-08 | Tenant scoping by discipline only | id-only lookups, e.g. SLS `teams.rs:114` checked one layer up | scoped repository signatures `(OrgId, Id)` for new code; `baukit-test` cross-tenant helper run per route |
| S-09 | No fuzz or property tests in baukit | 0 fuzz targets, 0 proptest; kassensturz is prior art | cargo-fuzz targets + proptest, time-boxed `fuzz` gate step |
| S-10 | No threat models | none in baukit; 3 products mention the term only | `baukit-threat-model` skill + `docs/threat-model.md` per repo |
| S-11 | No reusable auth negative tests | each product writes its own or none | `baukit-test` suite: expired, wrong iss/aud, alg none, HS256 confusion, unknown kid, tampered payload |
| S-12 | Template demo resource has no owner | `templates/backend/backend/migrations/0001_create_items.sql`: global `items` table, every authenticated user can list, rename and delete all rows. Routes still live in kassensturz, lesezeichen, schluesselbund; dead table left in eigenruhe, leitbild, tiefgang (W1-I) | template example scoped by the authenticated principal with a cross-tenant test, so the pattern agents copy is the safe one; products delete or scope `items` |

Checked and sound (do not re-review): JWT algorithm allowlist and kid/exp checks, constant-time compares,
credential vault (AES-256-GCM, random nonces, AAD), njs media-grant verifier, MCP origin + bearer checks,
Helm securityContext and NetworkPolicy, non-root Dockerfile template, no anyhow or `Box<dyn Error>` in
library APIs, no mutable globals, TS `strictTypeChecked` + `noUncheckedIndexedAccess` already on.

## Decisions

- D1 Pattern rules engine: ast-grep (single Rust binary, Rust and TS grammars, YAML rules, no account or
  telemetry). Rules live in `lint/ast-grep/` in baukit and ship with the CLI.
- D2 Token storage in `auth-web` (user, 2026-11-07): backend-for-frontend. Access token in memory only;
  baukit-auth gets token-exchange and refresh endpoints that keep the refresh token in an httpOnly
  `SameSite=Strict` cookie. API and web must be same-site in every product.
- D3 Agent instruction files (user, 2026-11-07): `CLAUDE.md` is the real file, `AGENTS.md` a symlink to it,
  as in baukit today.
- D4 `cognitive_complexity` stays off: clippy's own docs disown the metric, and with `-D warnings` there is no
  advisory level. `too_many_lines` (threshold 80) is the gate.
- D6 Worktrees (user, 2026-11-07): each parallel agent works in its own worktree under
  `../baukit-worktrees/<task>` on branch `task/qs-<task>`. After the merge the orchestrator removes the worktree,
  its `target/` and `node_modules`, and the branch.
- D5 Rollout: all baukit work ships as one minor release (0.19.0). Products upgrade one repo per agent,
  in a worktree from origin/main, gate green before push. Never touch a product's main checkout.

## Phase 1. Static analysis in baukit

Wave 1 (parallel, disjoint ownership):
W1-A `rust/` lints and fixes (P1.1 rust part, P1.9 Rust refactors);
W1-B `cli/` lints, fixes and the `lib.rs` split (P1.1 cli part, P1.9 CLI part);
W1-C `typescript/` eslint-config and shared guards (P1.6, P1.7);
W1-D supply chain and script linters for baukit's own CI (P1.3, P1.8, P2.5 baukit part).
Templates follow in wave 2, after the baukit-side config is settled.
Added with the raised cap: W1-E skills + baukit threat model (P2.10, P2.11), W1-F fuzz + proptest (P2.6, P2.7, no src edits),
W1-G ast-grep rules (P1.5 rules only), W1-H ADR 0006 browser token handling (P2.2 design), W1-I read-only
tenancy and rate-limit audit of the products (S-08).

- [x] P1.1 (rust/, cli/ and the backend template baseline with clippy.toml from W5-T1) Rust lint baseline: add the E1 set (expect_used, indexing_slicing, cast_*, too_many_lines at 80,
  allow_attributes(+_without_reason), missing_panics_doc, return_self_not_must_use, unused_async(+trait_impl),
  needless_pass_by_value, manual_let_else, struct_excessive_bools, print_stdout/stderr, disallowed_methods)
  plus a shared `clippy.toml` to `rust/`, `cli/` and the backend template; fix all violations in baukit.
  Acceptance: `make ci` green with `-D warnings`, no new `#[allow]`, each `#[expect]` has a reason.
- [x] P1.2 (F1a/F1b: every pub Result fn documented after tracing its Err sites; lint on in rust/ and cli/, not in the product baseline) `missing_errors_doc` on core, http, config, auth first, then the rest (255 docs).
- [~] P1.3 (rust/ and cli/ done in W1-D; template deny.toml open) cargo-deny: `allow-wildcard-paths`, full `check` for `rust/`, new `cli/deny.toml`, template `deny.toml`.
- [x] P1.4 (all seven rules merged: lints, file-size, suppressions, patterns, agent-docs, observability, template-files with `.baukit/template-files.toml` and the `scripts/quality-gate.local.sh` hook; the generated CI and strict gate run all seven since W5-T1) `baukit check` subcommand framework (also: `patterns` runs lint/ast-grep with ast-grep 0.51.1, skips
  `vendor/`, `target/`, `node_modules`, and rejects `ast-grep-ignore` comments without a `-- reason`) in the CLI, with rules:
  `lints` (baseline subset check for Cargo lint tables and clippy.toml), `file-size` (Rust/TS, shrinking
  allowlist in `baukit.toml`), `agent-docs` (Q-09), `template-files` (Q-07). Unit + CLI tests.
  Wired into `baukit doctor` and the template quality gate.
- [x] P1.5 (rules merged; CLI wiring is P1.4) ast-grep rules with tests: `as unknown as` outside tests, `derive(Debug)` on secret-named fields,
  `format!` starting with an SQL keyword, raw `reqwest::Client` outside adapters, `localStorage.setItem` with
  token-named keys. `baukit check patterns` runs them.
- [~] P1.6 `@baukit/eslint-config` package: typescript-eslint strictTypeChecked, react-hooks, jsx-a11y (web),
  max-lines-per-function 80, max-lines 500, max-depth 4, complexity 15 (warn), max-params 4,
  consistent-type-assertions, strict-boolean-expressions per E2. Used by `typescript/` and both templates.
  Fix the ~114 violations in baukit packages.
- [x] P1.7 (no `as unknown as` left in production code; the remaining test-file casts are DOM/Expo/RevenueCat test doubles; the last isRecord copies now import @baukit/type-guards) Shared type guards: one `isRecord`/validation helper module, remove the 10 copies and the
  avoidable `as unknown as` casts.
- [x] P1.8 (baukit in W1-D/W1-D2, templates in W5-T3) ruff + shellcheck for `scripts/` in baukit and the template gate; fix findings.
- [x] P1.9 (cli, Rust and navigation splits merged; template `api.rs` split into `compose_api` and `rate_limits.rs` in W5-T1) Refactors to meet the limits: split `cli/src/lib.rs`, `supervise_task`, `authorize_inner`,
  `worker.rs::execute`, navigation `native.tsx` components, template `api.rs::run` (composition builder).
- [ ] P1.10 `SuiteComposition` builder in baukit-suite; template uses it (Q-08). Crate part done (W4-G);
  template and product adoption pending.
- [x] P1.11 (F2: native rule agreeing with check-metric-names.py on shared fixtures, plus datasource and dashboard-coverage checks; product findings in the log, old linters go in P4.2) `baukit check observability` replacing the five product linters (Q-13).

## Phase 2. Security tooling, tests and skills

- [x] P2.1 (baukit-http layer in W2-2; HSTS field in W4-F; web CSP and fixture header checks in W5-T2; `@baukit/csp` as the one policy source plus COOP/CORP `same-origin` in vite preview and the chart in T4, COEP left unset, ADR 0006 section 7) `SecurityHeadersLayer` in baukit-http on by default (nosniff, Referrer-Policy, frame-ancestors via
  CSP for HTML, `Cache-Control: no-store` on authenticated JSON, HSTS behind config); CSP for the web template
  and the chart's edge config; header assertions in the fixture gate.
- [~] P2.2 (ADR tasks 1 and 2 merged: BFF in baukit-auth + ratelimit adapter, headers; task 3 auth-web/pwa-web after W1-C; task 4 templates; auth-web waits for W1-C; design: ADR 0006 accepted; implementation = its 4 tasks) auth-web token storage per D2, with a test that no storage write contains a refresh token.
- [x] P2.3 `IssuedApiToken` and every secret-holding type: redacting Debug / `SecretString`.
- [~] P2.4 (crate part merged in W4-F; the template `auth.allow_missing_azp` flag and the stale
  auth-providers.md:90 line are left for the backend template wave) Clerk missing-azp opt-in; negative tests for every token profile.
- [x] P2.5 (baukit in W1-D, template workflows and repo-lint in W5-T3) Supply chain: SHA-pin all actions in baukit and the template workflows, renovate digest pinning,
  zizmor + actionlint step, gitleaks with fixture allowlist, osv-scanner over Cargo.lock and pnpm-lock.yaml.
- [~] P2.6 (6 targets and the gate lane merged; template scaffold open; baukit has no sync push decode because the wire protocol is product-owned, so push batch fuzzing moves to the P4.2 rollout) Fuzzing: `rust/fuzz` with targets for JWT header/claims decode, sync push batch decode, event
  envelope, MCP JSON-RPC dispatch, idempotency-key and locale header parsers; time-boxed `fuzz` gate step
  (nightly, 60 s per target locally, longer on schedule). Template gets a `backend/fuzz` scaffold and the step.
- [x] P2.7 (vault, media grants, HLC merged; F5 added a two-owner proptest over the crate's revision and purge calls on Postgres, which found purge deleting another owner's row with the same id; `TombstoneTable` delete now binds owners as `$2`; push/pull isolation stays product-owned and moves to the P4.2 rollout) Property tests: credential vault (roundtrip, any AAD change fails), media grants (Rust signer and
  njs verifier agree, single-byte mutations rejected), sync merge (idempotent, owner isolation).
- [x] P2.8 (crate helpers merged; template `tests/support/owner_scoping.rs` in W5-T1) `baukit-test` security helpers: auth negative suite and `assert_cross_tenant_denied` that walks the
  OpenAPI routes; template integration test that uses both.
- [x] P2.9 (baukit in W4-C; template `clippy.toml` in W5-T1) clippy `disallowed-methods` for raw reqwest outside an allowlisted adapter module (template
  `clippy.toml`).
- [x] P2.10 (threat-model and security-testing in W1-E, code-conformance in F3) Skills in `agent-skills/skills/`: `baukit-threat-model` (data classes, trust boundaries,
  STRIDE per boundary, threat → control → test table, output `docs/threat-model.md`),
  `baukit-security-testing` (when and how to add fuzz, property and negative auth tests),
  `baukit-code-conformance` (fixing lint and check failures without suppressing them).
- [x] P2.12 (W5-T1: `owner_id` column, owner in every WHERE, server-generated UUIDv7 ids, second-owner test) Template `items` example: owner column, `(owner, id)` in every query, cross-tenant test in the
  generated backend (S-12). Lands with the template wave.
- [x] P2.13 (merged with W1-A; Docker test `foreign_owner_lookup_returns_none_without_accounting_failures`) baukit-suite `find_link_for_owner` (`adapters/postgres/mod.rs:539`) loads by id, runs
  `refresh_failures` on the foreign id, then filters the owner in Rust. Move `user_id` into the WHERE and refresh
  only after the owner matches. After W1-A merges (it owns the crate's lint fixes).
- [x] P2.11 baukit's own `docs/threat-model.md`, written with the new skill.

## Phase 3. Agent instructions

- [x] P3.1 baukit `CLAUDE.md`: coding rules (no panics in prod paths, `#[expect]` with reason, size limits,
  scoped queries, egress only, no tokens in web storage, threat model on new boundaries), the new checks,
  and remove the duplicated `ci-gate.sh` paragraph.
- [x] P3.2 (W5-T3; markers are `baukit:agent-rules:start/end`, filled from lint/agent-rules.md at generation) Template managed block `<!-- baukit:begin -->`/`<!-- baukit:end -->` in the generated CLAUDE.md
  with the shared rules; `baukit upgrade` refreshes it; `baukit check agent-docs` verifies it and the D3
  file convention.
- [x] P3.3 (agent-skills/README.md lists all 14 skills; the generated CLAUDE.md has a skills section since W5-T1) Skills index and install docs updated.

## Phase 4. Release and rollout

- [x] P4.1 (0.19.0 npm by hand, 0.19.1 through the pipeline) Full baukit gate (`sh scripts/ci-gate.sh`, `--include-ignored`, `make generated-fixture-gate`
  all flavors, MSRV, cargo deny, version coherence), release 0.19.0.
- [ ] P4.2 Product upgrades, one agent per repo, four at a time. Per repo: bump to 0.19.0, adopt the lint
  baseline and clippy.toml, `@baukit/eslint-config`, deny.toml, gitleaks/zizmor/osv steps, managed
  instruction block, fix every violation (shrinking allowlists only for size limits), migrate raw reqwest to
  egress, SHA-pin actions, write `docs/threat-model.md`, add fuzz targets for its own parsers, gate green.
  - [ ] eigenruhe  - [ ] hebkit  - [ ] leitbild  - [ ] redemut
  - [ ] schlauzug  - [ ] tiefgang  - [ ] solo-leveling-system  - [ ] lesezeichen
  - [ ] schluesselbund  - [ ] kassensturz  - [ ] runtime-analyzer  - [ ] architecture-health-platform
  - Product-specific items gathered from baukit work packages:
    - `TombstoneTable.delete` binds owners as `$2` and deletes `(owner_id, id)` pairs (F5): eigenruhe,
      hebkit, tiefgang, kassensturz. Each product also gets its own push/pull owner-isolation test and a fuzz
      target for its push batch decode, since baukit-sync leaves the wire format to products.
    - Clerk products that sign in from mobile opt in to `allow_missing_azp` (W4-F).
    - OAuth provider linking moves to `baukit_integrations::oauth_state` (F4): solo-leveling-system,
      lesezeichen and every other product with an integration callback. Suite consumers mount public routes
      through `PostgresSuite::router` or `limited_router`; the plain `adapters::http::router` is gone.
    - Suite builder adoption (reports/suite-builder-adoption.md) and the per-product tenancy findings
      (reports/I-tenancy-ratelimit.md).
  Tenancy and rate-limit fixes from W1-I (`docs/quality-security/I-tenancy-ratelimit.md`), verified by the
  orchestrator unless marked:
  - kassensturz, lesezeichen, schluesselbund: delete or owner-scope the template `items` routes (S-12).
  - redemut: sync push takes `device_id` from the client (`redemut-services/src/lib.rs:1299`) and the FK is
    global (`migrations/20260901000000_storage_usage.sql:17`), so user B can attach events to A's device and
    block A's erasure. Composite FK `(user_id, device_id)` plus a check.
  - runtime-analyzer: one global `anonymous:auth` bucket (`finops-api/src/lib.rs:278`) lets one client lock
    out all sign-ins. Key it per client IP.
  - kassensturz (9 tables), hebkit (2): sync upserts need the owner in `ON CONFLICT ... WHERE`. Unverified.
  - solo-leveling-system (verified): `GET /guilds/{id}` and its members, progression, quests and competitions
    ignore private visibility (`services/src/groups.rs:1015` checks only `disbanded_at`); `GET /parties/{id}`
    returns any party with members (`services/src/groups.rs:1226`); cohort `add_member` enrols a foreign
    `userId` in the arc before the FK rejects the member row (`services/src/orgs.rs:1934-1956`); the
    integration callback trusts a signed, TTL-bound `state` that is neither single-use nor bound to the
    browser, so an attacker-started authorize URL links the victim's provider account to the attacker
    (`v1/integrations.rs:355-388`). Minor: `POST /challenges` takes any `guildId`.
  - eigenruhe: no missing scoping. Sync `id_is_owned_or_missing` (`eigenruhe-postgres/src/sync.rs:918`) races on
    a concurrent first insert of one id by two owners: the loser gets `Accepted` with nothing written.
  - schluesselbund: family routes and platform family members check membership in Rust after an id load;
    `require_world` runs before the membership check (existence oracle). Unverified.
  - eigenruhe (verified: `eigenruhe-bin/src/bin/api.rs:445-447` merges suite and platform outside
    `baukit_ratelimit::layers`), lesezeichen: `/suite/*` and `/platform/inbound` are not rate limited;
    kassensturz has no limiter on `/platform/inbound`; schluesselbund never limits invalid `/platform/v1`
    tokens (the last three unverified).
  - Every product: a two-principal test per id route (current coverage ranges from 2 of 18 to 42 of 64).
- [ ] P4.3 Cross-repo conformance sweep: `baukit check` green in all 12, counts table recorded below.

## Log

- 2026-11-07: plan created, exploration E1 to E4 started.
- 2026-11-07: E4 report verified. Confirmed E4-01 (localStorage tokens, auth-web index.ts:428), E4-02 (no security
  headers anywhere), E4-03 (0/80 SHA pins in baukit), E4-04 (deny.toml in 3 of 12 products), E4-06, E4-07
  (Clerk accepts missing azp, verifier.rs:1033). Rejected E4-05's "0 cross-tenant tests" counts: the grep used
  five names only; a broader sweep finds 7 to 73 candidate files per repo. Tenant isolation stays a hardening item
  that needs a per-route audit, not a confirmed gap.
- 2026-11-07: E1 report verified. Re-ran `cargo deny check bans sources` (12 wildcard errors, 15 duplicates as
  reported), checked file and function references, the 11 `#[allow]` attributes and every proposed lint name
  against `cargo clippy --explain`. Note: clippy's own docs disown `cognitive_complexity`, so `too_many_lines`
  is the primary size limit and complexity stays advisory.
- 2026-11-07: E2 report verified. Confirmed strictTypeChecked + noUncheckedIndexedAccess + exactOptionalPropertyTypes
  already on, 17 `as unknown as` in src, `isRecord` copied in 10 packages, no react-hooks plugin in any
  typescript/ eslint config (templates have it), purchases-core without node_modules (stale install, ignored).
  Navigation's own lint script passes, so TSX files are covered by per-package configs.
- 2026-11-07: E3 report verified. Confirmed lint drift (redemut only `clippy::all`, SLS only `all = "deny"`,
  hebkit edition 2021), god files (redemut-api 8491, schlauzug-api 7420, AHP limits.rs 4369), template
  `run` 205 lines, 12 distinct `quality-gate.sh` hashes, AHP symlink vs hebkit check, eigenruhe CLAUDE.md/AGENTS.md
  line 200 contradiction. Its function-length counter is a heuristic; P1.1 re-measures with clippy.
- 2026-11-07: wave 1 started: W1-A..D, then W1-E..I after the cap went to 10.
- 2026-11-07: W1-E merged (a509243b). Checked every helper the skills cite (baukit_test::jwt, egress
  StaticResolver/BlockedAddress, ApiError::not_found) and every test cited in docs/threat-model.md: all exist.
  Worktree and branch removed (no build cache was created).
- 2026-11-07: W1-H ADR 0006 sent back: grace window not implementable with hashed secrets, CSP breaks Expo/RNW
  inline styles and misses media origins, no rate limiting on /auth/session/*.
- 2026-11-07: ADR 0006 amended (grace token column + FOR UPDATE, per-flavor CSP, rate-limit buckets,
  credentials scope, Keycloak idle note), accepted and merged. Worktree removed.
- 2026-11-07: W2-1 (ADR 0006 task 1, baukit-auth BFF) and W2-2 (task 2, baukit-http headers) started in
  parallel with W1-A; both check against the new lint set via CLIPPY_CONF_DIR without editing the lint table.
- 2026-11-07: Full Rust gate on main after W1-A: fmt, clippy `--all-features -D warnings`, 105 test suites
  with `--include-ignored`, 0 failed. W2-1 merged after round 3 (502 `idp_invalid_response`); clippy and the
  auth and ratelimit suites re-run on main with Docker. Orchestrator added the auth, ratelimit and suite
  changelog entries.
- 2026-11-07: W1-A merged after its second round: re-ran fmt, clippy `--all-features -D warnings` and the
  P2.13 Docker test, and reviewed the jitter, suppression and suite diffs. 93 suppressions with reasons; about 60 of
  them are the same file-level allow in integration tests, where allow-*-in-tests does not reach helper
  functions. W2-1 round 2 verified (ID token signature through `OidcVerifier`, `StoreBrowserSessionRateLimiter`
  in baukit-ratelimit, auth and ratelimit tests re-run with Docker). It now rebases onto the lint set.
- 2026-11-07: W1-F merged after re-running its 3 property suites and 15 s smoke runs of jwt_verify and
  mcp_jsonrpc. The orchestrator wired a `fuzz` gate lane and CI job (fmt, clippy, 60 s per target).
  W1-D2 merged: osv ignores 46 to 37, all on typescript/pnpm-lock.yaml or with no fix; ruff and shellcheck
  cover every tracked file except templates/. W1-B merged: fmt, clippy and every test including Docker ones
  re-run, snapshots unchanged, 5 suppressions with reasons.
  Sent back: W1-A (merge main, lint the new headers and property tests, P2.13), W1-C (a disable comment the
  report claimed had a reason did not have one; enforce `require-description`; type-guards wiring for
  fixtures; TS lock bumps), W2-1 (verify the ID token signature, baukit-ratelimit adapter). ADR 0006 now states
  the real Quota semantics (rpm + burst admitted).
  Open: templates/ Python and shell lint (template wave).
- 2026-11-07: W2-2 merged (ADR 0006 task 2). Re-read the layer against ADR section 7, re-ran its 9 tests, and
  grepped rust/, cli/ and templates/ for HTML-serving routes (none, so the API CSP on every response is safe).
  Follow-up: `hsts_max_age` in baukit-config `HttpConfig`, so products set HSTS from config (template wave).
- 2026-11-07: W1-D merged after re-running its 7 gate steps, checking 14 action pins against upstream tags and
  diffing version-coherence output. 46 time-boxed osv ignores and narrowed ruff/shellcheck scopes went back as
  W1-D2 (example lock bumps, repo-wide script lint). typescript/pnpm-lock bumps wait for W1-C.
- 2026-11-07: W1-I audit in. Verified: S-12 items, redemut device FK, RA shared auth bucket, baukit-suite
  owner filter in Rust (P2.13). Rejected: kassensturz aggregator-consent DoS (the composite FK at
  `0046_consent_expiry.sql:38` blocks the insert). Seven products got spot checks only, so P2.8's per-route
  helper has to close the gap.
- 2026-11-07: W1-G merged. Re-ran `ast-grep test` (7/7) and scans of baukit and hebkit (counts match).
  Sampled secret-debug hits: real tokens plus one borderline (`preview_token`). `rust-sql-unscoped-by-id`
  is hint-only (455 hits, mostly global tables). Baseline hits: ts-double-cast 17 (P1.7), raw client 6,
  secret-debug 2, token storage 1 (P2.2). Product totals are in the W1-G report numbers in the merge commit's
  lint/README.md context; rollout must fix them.
- 2026-11-07: findings catalog and phases written. User chose D2 = BFF refresh cookie, D3 = CLAUDE.md + symlink.
- 2026-11-07: W1-C merged after a rework round.
  - Re-ran the TS gate on main: build 25/25, format 26/26, lint 26/26, test 51/51. cli: clippy clean, 254
    tests passed. osv: clean with 5 ignores. Versions coherent.
  - A planted bare `eslint-disable` fails `require-description`.
  - glob 14.0.2 and minimatch 11.0.1 are forced through overrides: OSV lists the fixes only in those majors.
  - The navigation re-export regression was found by the mobile fixture and fixed.
- 2026-11-07: W3-1 `baukit check` merged locally (not pushed). Planted violations are caught (bare
  `#[allow]`, bare `eslint-disable`, a 1001-line file).
  - Bug the agent missed: the walker ignores .gitignore, so `deploy/platform/.local-state` and
    `rust/target-msrv` are scanned. Sent back with the `examples/minimal-api` lint adoption.
  - main stays local until `baukit check` is green.
  - Open patterns findings on main went to the W4 batch: 6 raw HTTP clients, 3 secret-Debug, 10 TS double
    casts, and 1 web-storage token (auth-web).
- 2026-11-07: tenancy audit parts for SLS, eigenruhe and schluesselbund verified. The P4.2 list is updated.
  - The template merges `platform.router` outside the limiter too. That goes into the template wave.
  - SLS OAuth linking CSRF: baukit-integrations has no OAuth state helper. Consider one (state bound to a
    browser cookie, single-use) before the SLS rollout.
- 2026-11-07: W4 batch started, 7 agents:
  - B: P2.3 secret Debug.
  - C: egress migration plus P2.9.
  - D: TS double casts.
  - E: ADR task 3 BFF client.
  - F: P2.4 Clerk azp plus the HSTS config field.
  - G: P1.10 suite builder plus a limiter on unauthenticated suite routes.
  - H: P2.8 test helpers.
- 2026-11-07: W4-D merged.
  - 9 `ts-double-cast` findings are gone. analytics-core probes globals as `unknown` with typeof checks.
    sync-client wraps host interval ids in opaque handles through a WeakMap.
  - No suppressions. Main: TS lint 26/26, tests pass for the affected packages.
  - Leftover: a test-only `readFileSync as unknown as` in `sync-client/src/hlc.test.ts:100`. The rule skips
    tests.
- 2026-11-07: P3.1 done. Rewrote `lint/agent-rules.md`: gate, no prod panics, suppressions, file size,
  hexagonal, TS guards, owner-scoped SQL plus a second-principal test, every route in the limiter, egress,
  secrets, no browser token storage, deny/osv/SHA pins, threat model. Synced with `--fix`. Removed the
  duplicated gate paragraph from CLAUDE.md.
- 2026-11-07: W5-T3 (template repo-level: SHA pins, security steps, `baukit check` in the generated gate,
  managed block, AGENTS.md symlink) started.
- 2026-11-07: W3-1 rework merged.
  - The walker uses the `ignore` crate, so .gitignore is honoured in every rule. examples/minimal-api adopts
    the lint baseline with real fixes.
  - The report claimed "no new crates". In fact 7 came in (ignore, walkdir, crossbeam-*, same-file,
    winapi-util). cargo deny is clean.
  - Main `baukit check`: file-size, suppressions and agent-docs ok. 2 lints findings (template eslint configs,
    T2) and 10 patterns findings (W4-B, W4-C, W4-E) remain.
- 2026-11-07: W4-E merged (ADR 0006 task 3).
  - `BrowserSessionClient` keeps the access token in memory and coordinates refresh with Web Locks and
    BroadcastChannel. `OidcClient` is cut down to PKCE redirects. pwa-web makes `/auth/session/` and
    Authorization requests network-only.
  - Main: full TS gate green. Browser storage holds only the account hint, the PKCE transaction and the
    force-login marker. The `ts-token-web-storage` finding is gone.
  - Accepted deviation: refresh on load happens only when an account hint exists.
  - Known break: the OIDC web template fails to compile until ADR task 4. Started W5-T2 for that: BFF mount,
    web client swap, CSP, e2e, and `@baukit/eslint-config` in the web and mobile templates.
- 2026-11-07: W4-H merged (P2.8 crate part).
  - `assert_cross_tenant_denied` walks every OpenAPI operation that has a path parameter. A missing fixture
    id or a stale or blank exclusion fails. `assert_auth_negative_suite` runs 5 bad credentials against
    secured operations.
  - 2 reasoned `print_stdout` expects; rust-expect max raised to 31. The 6 self-tests pass on main.
  - The template test waits for P2.12 (items owner scoping). The ready version is in
    reports/test-helpers-template.rs.
  - Fixed on main: `verify_exchanged_id_token` is now gated behind `browser-session`.
    `cargo clippy -p baukit-auth` without the feature failed on dead code; the workspace build hid it
    through feature unification.
- 2026-11-07: W4-G merged (P1.10 crate part).
  - `PostgresSuite::builder` replaces the `SuiteComposition` copied into each product bin.
    `PostgresSuite::router` puts callback, exchange and revoke behind a per-IP bucket: 60 a minute, burst 10,
    fail closed. Signed inbound events keep their per-link and per-source buckets in the ingest service.
  - The plain `adapters::http::router` still mounts the routes without that bucket. It stays until the
    template and the products switch to the builder.
  - Review gap fixed on main: the report had no test for the fail-closed path. Added
    `unavailable_limiter_rejects_public_routes_by_default`.
  - Open: template and product adoption (backend template wave). Adoption notes in
    reports/suite-builder-adoption.md.
- 2026-11-07: W4-B merged (P2.3).
  - Redacting `Debug` impls: `IssuedApiToken` (also lost `PartialEq`/`Eq`), `SessionRow`, `KeycloakLogin`,
    and the link-code request types `LinkCompleteRequest` and `LinkCallbackQuery`.
  - `IssuedApiToken` and `GeneratedApiToken` moved to `api_token/issued.rs`; api_token.rs allowlist
    1977 -> 1910.
  - The `rust-secret-debug` rule now also matches more field names (passphrase, credentials,
    code_verifier, *_key), byte arrays and `Zeroizing` types. The 3 findings on main are gone.
  - I checked the sweep claims against the diff. Nothing else derives Debug on a secret.
- 2026-11-07: W4-F merged (P2.4 crate part, P2.1 HSTS field).
  - `ClerkVerifier` now rejects tokens without `azp`. Clerk leaves `azp` out when the request has no Origin,
    which is the case for native apps. `allow_missing_authorized_party()` turns acceptance back on. A present
    `azp` must still be in the allowlist.
  - New `tests/token_profiles.rs`: Oidc, Clerk, ClientBound and ScopeBound each get one valid token that must
    verify, then 9 or 10 attacks, each checked for its exact error variant.
  - `HttpConfig.hsts_max_age`: one year outside Local unless set. `0` turns it off. baukit-config/lib.rs
    dropped from the allowlist (961 lines).
  - Main after the merge: fmt, clippy (all features and without features for auth, config and http), 475
    tests with Docker, and file-size, suppressions and agent-docs are all green.
  - Rollout impact: 10 of the 12 products ship Clerk as a provider option and a mobile app. All except
    runtime-analyzer and architecture-health-platform. The backend template wave adds
    `auth.allow_missing_azp`, default on when `context.mobile`, and P4.2 wires it per product. Otherwise a
    Clerk deployment rejects Expo tokens.
- 2026-11-07: W4-C merged (P2.9 baukit part).
  - auth (discovery and JWKS), erasure, MCP introspection and push now send outbound HTTP through
    `GuardedClient`. Without an override, each client can reach only the exact scheme, host and port that was
    configured. That blocks every other host, plus metadata and link-local addresses, which is stricter than
    the old raw clients.
  - `OidcConfig::with_egress_policy` overrides the policy. A JWKS URL named by discovery on another host gets
    its own exact-endpoint allowance.
  - clippy `disallowed-methods` now covers `reqwest::Client::{new,builder}`, `ClientBuilder::new` and
    `reqwest::get`. 3 reasoned expects: the egress builder, baukit-test's `test_http_client`, and the rmcp
    example.
  - The `rust-raw-http-client` pattern findings are gone. `baukit check patterns` is green on main.
  - Combining the W4-F and W4-H merges with this one broke the clippy gate. W4-H's `security_probe.rs` and
    W4-F's `token_profiles.rs` used raw reqwest, so I fixed both on main with `test_http_client()`. Also synced
    examples/minimal-api/clippy.toml and lowered verifier.rs to 1670.
  - Main gate:
    - fmt, clippy (all features and without features per crate) and cli clippy: green.
    - 1292 workspace tests with Docker, 0 failed. MSRV 1.95 and cargo deny: green.
    - Fixture backend flavor green. The auth flavor fails only on the web template (`OidcClient` is not a
      `ProviderClient`), the known break until W5-T2.
  - `baukit check`: everything green except `lints` (2 template eslint configs, W5-T2).
  - Backend template wave brief written: prompts/W5-T1-template-backend.md. It starts after W5-T2 merges
    (both edit the backend bin crate).
  - Clerk fixture on main: 39 backend tests pass. Web fails in `src/sign-in/local.ts` with the same
    `OidcClient` vs `ProviderClient` mismatch. Sent to W5-T2.
- 2026-11-07: W5-T3 merged after one rework (P2.5 template part, P3.2, P1.8 template part).
  - Generated workflows:
    - every action is pinned to a verified SHA, with `persist-credentials: false`;
    - PR inputs reach the shell through `env:`;
    - new `repository-lint` job: actionlint, zizmor, gitleaks, osv, ruff, shellcheck;
    - `baukit check agent-docs file-size suppressions` runs in CI and the strict gate.
  - Generated CLAUDE.md gets the managed block from lint/agent-rules.md, and AGENTS.md is a symlink (D3).
  - First report: the new lint steps were red in every generated product, and the fixture gate didn't run
    them. Sent back. The rework:
    - fixed the ruff and shellcheck findings in the template scripts with real splits;
    - added 2 reasoned S310 per-file ignores and 2 `source=/dev/null` lines;
    - bumped vite and added glob/minimatch overrides;
    - added dated osv ignores (until 2027-02-05) for 4 advisories with no usable fix. decode-uri-component
      0.5 is ESM-only and breaks expo-router's query-string;
    - fixed two repo-lint.sh bugs (nested git repo, actionlint path);
    - added repo-lint to `generated-fixture-gate.sh`.
  - The rework claimed `baukit check` counts gitignored `dist/`. Not reproducible on main or in a fresh
    worktree with a planted `dist/probe.js`: suppressions ok.
  - The report's green run of the combined, auth, mcp and platform flavors came from a branch cut before
    W4-E. On main those flavors wait for W5-T2.
  - Main after the merge: cli fmt and clippy green, 292 cli tests, `baukit check` green except `lints`
    (W5-T2).
  - Fixture gate on main with the merged binary: backend, web, mobile, mobile-pwa and platform-standalone pass.
    Repo-lint ran in every fixture: zizmor no findings, gitleaks no leaks, osv, ruff and shellcheck clean.
- 2026-11-07: W5-T2 first report (ADR 0006 task 4, eslint config in templates). Not merged yet.
  - Delivered:
    - backend BFF mount and web `BrowserSessionClient`;
    - CSP from the build with script hashes, checked against the served header;
    - 6 browser-session stack e2e specs and a header check in the fixture gate;
    - `@baukit/eslint-config` in web and mobile, with real splits and 0 suppressions;
    - all 11 flavors passed at least once.
  - Sent back. The clerk stack test "two tabs refresh once" fails in about 2 of 5 runs: an auth-web race in
    `refreshUnlessSettled`. The BroadcastChannel message can land after the lock is released, so the waiting
    tab refreshes again. The second refresh stays inside the 10 s reuse grace, so nobody is logged out. It
    still costs an extra request and makes the gate flaky.
  - The rework also covers a dev-dependency slot for mobile, the mcp-remote TS filter, and surfacing the
    swallowed `session()` error.
  - Follow-ups (not in this wave): the CSP `policy.ts` duplicated between web and mobile, and COOP/COEP
    headers in the chart nginx config.

### 2026-11-07 follow-up batch (F1 to F5)

- F1a/F1b: about 290 `# Errors` sections. The first part-A pass named a variant `encrypt` cannot return; the
  traced rework fixed 48 of 161 sections. `missing_errors_doc` is now on in rust/ and cli/. rustdoc runs with
  `-D warnings` in the rust-check and cli-check steps.
- F2: `baukit check observability` and `template-files`. Product findings (none reported by the old linters):
  eigenruhe 20 datasource (fixed uid `prometheus`), kassensturz 6 metrics on no dashboard, AHP 81 coverage +
  9 undeclared `DS_PROMETHEUS`, lesezeichen 14, leitbild 4, schluesselbund 16 coverage gaps. promtool stays a
  separate step. Generated products get `.baukit/template-files.toml` and a `.gitleaks.toml` that allowlists it.
- F3: baukit-code-conformance skill and the skills index.
- F4: `baukit_integrations::oauth_state` (single-use, browser-bound state with PKCE, Postgres consume race test,
  fuzz target, threat-model rows B-18 to B-20). The unlimited suite router is gone.
- F5: owner-isolation proptest found `purge_tombstone_batch` deleting another owner's row with the same id; the
  delete SQL now gets owners as `$2`.
- Gotcha: the kache rustc wrapper can serve a stale `baukit` binary after new template files land
  (`include_dir!`); `cargo clean -p baukit-cli` fixes it.
- T4: `@baukit/csp` replaces the two template copies of `policy.ts` (vectors checked byte-equal against the old
  output by the orchestrator). COOP and CORP `same-origin` in vite preview and the chart, COEP unset. The
  agent's ADR note claimed a COOP-free callback page would keep the `SilentTokenBroker` popup's `opener`; the
  popup inherits the opener's COOP, so the issuer hop already cuts it. Corrected: a product that adds the popup
  needs `same-origin-allow-popups` on its app pages and no COOP on the callback.
- W5-T1: owner-scoped items, a rate limiter on every template route, the Clerk `allow_missing_azp` opt-in for
  mobile, the strict lint baseline, a cargo-fuzz scaffold, and cargo-deny, fuzz and all seven `baukit check`
  rules in the generated gate. Orchestrator fix: `/mcp` sat outside every limiter for invalid tokens, because
  baukit_mcp keys its own limiter on the verified caller. The MCP router is now wrapped in the IP limiter.
  Products without auth use an in-memory IP limiter, so each replica keeps its own buckets.
- Release prep: `@baukit/type-guards`, `@baukit/eslint-config` and `@baukit/csp` have never been on npm, and
  npm sets up trusted publishing only for existing packages. The user publishes 0.19.0 once by hand, then
  configures trusted publishers. Follow-up: the baukit-upgrade skill still describes git-tag pins; products
  pin registry versions.
- Product rollout brief: prompts/P4.2-rollout.md. architecture-health-platform's main checkout has 4 deleted
  tracked fixtures under `node_modules/` and `target/` paths, probably from a broad cleanup. The brief allows only
  `git clean -ffdX`.
- 2026-11-08: hosted CI on main had been red since T4. The seven generated-fixture jobs hit the AppArmor userns
  block in `with-loopback-network.sh` (the Dexie job already lifted it, the fixture job did not). The fuzz job
  had never passed, because install-action ships a musl cargo-fuzz, which defaults to the musl target, and ASan
  rejects that. Fixed in PR #2 (49b2dad9): a guarded sysctl step, plus `--target` set to the host triple in
  `scripts/fuzz.sh` and in the strict template's fuzz step. PR checks 37/37 green.
- v0.19.0 release run 42431811804 died in "Verify tagged train" with exit 137. `pnpm check --concurrency=4`
  passed the flag to `lint:root`, so turbo ran type-aware eslint at default concurrency and ran out of memory.
  No crate reached crates.io. The step now calls `ci-gate.sh --step ts-check` with 4 workers (805c5710).
  0.19.1 is the first train published to both registries by the workflow, and it validates npm trusted
  publishing. Compatibility-matrix v0.19.1 tells 0.18.x products to go straight to 0.19.1.
- Follow-up done: the baukit-upgrade skill and baukit-conventions.md describe registry pins (505c7f08).
