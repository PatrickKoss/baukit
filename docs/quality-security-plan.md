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

- [~] P1.1 Rust lint baseline: add the E1 set (expect_used, indexing_slicing, cast_*, too_many_lines at 80,
  allow_attributes(+_without_reason), missing_panics_doc, return_self_not_must_use, unused_async(+trait_impl),
  needless_pass_by_value, manual_let_else, struct_excessive_bools, print_stdout/stderr, disallowed_methods)
  plus a shared `clippy.toml` to `rust/`, `cli/` and the backend template; fix all violations in baukit.
  Acceptance: `make ci` green with `-D warnings`, no new `#[allow]`, each `#[expect]` has a reason.
- [ ] P1.2 `missing_errors_doc` on core, http, config, auth first, then the rest (255 docs).
- [~] P1.3 cargo-deny: `allow-wildcard-paths`, full `check` for `rust/`, new `cli/deny.toml`, template `deny.toml`.
- [ ] P1.4 `baukit check` subcommand framework in the CLI, with rules:
  `lints` (baseline subset check for Cargo lint tables and clippy.toml), `file-size` (Rust/TS, shrinking
  allowlist in `baukit.toml`), `agent-docs` (Q-09), `template-files` (Q-07). Unit + CLI tests.
  Wired into `baukit doctor` and the template quality gate.
- [ ] P1.5 ast-grep rules with tests: `as unknown as` outside tests, `derive(Debug)` on secret-named fields,
  `format!` starting with an SQL keyword, raw `reqwest::Client` outside adapters, `localStorage.setItem` with
  token-named keys. `baukit check patterns` runs them.
- [~] P1.6 `@baukit/eslint-config` package: typescript-eslint strictTypeChecked, react-hooks, jsx-a11y (web),
  max-lines-per-function 80, max-lines 500, max-depth 4, complexity 15 (warn), max-params 4,
  consistent-type-assertions, strict-boolean-expressions per E2. Used by `typescript/` and both templates.
  Fix the ~114 violations in baukit packages.
- [~] P1.7 Shared type guards: one `isRecord`/validation helper module, remove the 10 copies and the
  avoidable `as unknown as` casts.
- [~] P1.8 ruff + shellcheck for `scripts/` in baukit and the template gate; fix findings.
- [ ] P1.9 Refactors to meet the limits: split `cli/src/lib.rs`, `supervise_task`, `authorize_inner`,
  `worker.rs::execute`, navigation `native.tsx` components, template `api.rs::run` (composition builder).
- [ ] P1.10 `SuiteComposition` builder in baukit-suite; template uses it (Q-08).
- [ ] P1.11 `baukit check observability` replacing the five product linters (Q-13).

## Phase 2. Security tooling, tests and skills

- [~] P2.1 (W2-2) `SecurityHeadersLayer` in baukit-http on by default (nosniff, Referrer-Policy, frame-ancestors via
  CSP for HTML, `Cache-Control: no-store` on authenticated JSON, HSTS behind config); CSP for the web template
  and the chart's edge config; header assertions in the fixture gate.
- [~] P2.2 (W2-1 baukit-auth BFF running; auth-web waits for W1-C; design: ADR 0006 accepted; implementation = its 4 tasks) auth-web token storage per D2, with a test that no storage write contains a refresh token.
- [ ] P2.3 `IssuedApiToken` and every secret-holding type: redacting Debug / `SecretString`.
- [ ] P2.4 Clerk missing-azp opt-in; negative tests for every token profile.
- [~] P2.5 Supply chain: SHA-pin all actions in baukit and the template workflows, renovate digest pinning,
  zizmor + actionlint step, gitleaks with fixture allowlist, osv-scanner over Cargo.lock and pnpm-lock.yaml.
- [~] P2.6 Fuzzing: `rust/fuzz` with targets for JWT header/claims decode, sync push batch decode, event
  envelope, MCP JSON-RPC dispatch, idempotency-key and locale header parsers; time-boxed `fuzz` gate step
  (nightly, 60 s per target locally, longer on schedule). Template gets a `backend/fuzz` scaffold and the step.
- [~] P2.7 Property tests: credential vault (roundtrip, any AAD change fails), media grants (Rust signer and
  njs verifier agree, single-byte mutations rejected), sync merge (idempotent, owner isolation).
- [ ] P2.8 `baukit-test` security helpers: auth negative suite and `assert_cross_tenant_denied` that walks the
  OpenAPI routes; template integration test that uses both.
- [ ] P2.9 clippy `disallowed-methods` for raw reqwest outside an allowlisted adapter module (template
  `clippy.toml`).
- [~] P2.10 (threat-model + security-testing done in W1-E; code-conformance waits for the checks) Skills in `agent-skills/skills/`: `baukit-threat-model` (data classes, trust boundaries,
  STRIDE per boundary, threat → control → test table, output `docs/threat-model.md`),
  `baukit-security-testing` (when and how to add fuzz, property and negative auth tests),
  `baukit-code-conformance` (fixing lint and check failures without suppressing them).
- [x] P2.11 baukit's own `docs/threat-model.md`, written with the new skill.

## Phase 3. Agent instructions

- [ ] P3.1 baukit `CLAUDE.md`: coding rules (no panics in prod paths, `#[expect]` with reason, size limits,
  scoped queries, egress only, no tokens in web storage, threat model on new boundaries), the new checks,
  and remove the duplicated `ci-gate.sh` paragraph.
- [ ] P3.2 Template managed block `<!-- baukit:begin -->`/`<!-- baukit:end -->` in the generated CLAUDE.md
  with the shared rules; `baukit upgrade` refreshes it; `baukit check agent-docs` verifies it and the D3
  file convention.
- [ ] P3.3 Skills index and install docs updated.

## Phase 4. Release and rollout

- [ ] P4.1 Full baukit gate (`sh scripts/ci-gate.sh`, `--include-ignored`, `make generated-fixture-gate`
  all flavors, MSRV, cargo deny, version coherence), release 0.19.0.
- [ ] P4.2 Product upgrades, one agent per repo, four at a time. Per repo: bump to 0.19.0, adopt the lint
  baseline and clippy.toml, `@baukit/eslint-config`, deny.toml, gitleaks/zizmor/osv steps, managed
  instruction block, fix every violation (shrinking allowlists only for size limits), migrate raw reqwest to
  egress, SHA-pin actions, write `docs/threat-model.md`, add fuzz targets for its own parsers, gate green.
  - [ ] eigenruhe  - [ ] hebkit  - [ ] leitbild  - [ ] redemut
  - [ ] schlauzug  - [ ] tiefgang  - [ ] solo-leveling-system  - [ ] lesezeichen
  - [ ] schluesselbund  - [ ] kassensturz  - [ ] runtime-analyzer  - [ ] architecture-health-platform
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
- 2026-11-07: findings catalog and phases written. User chose D2 = BFF refresh cookie, D3 = CLAUDE.md + symlink.
