# E3: product code smells and cross-repo conformance

Scope: 12 products under `/home/patrick/projects/`. Static methods only (rg, find, python). All commands run from `/home/patrick/projects`. Scratch scripts live in `/home/patrick/.cache/baukit-orchestrator/quality-security/tmp/`:

- `matrix.sh`: conformance matrix fields
- `counts.py`: per-repo escape-hatch counts split test/non-test, size outliers, Rust long-function count (output in `counts.out`, `counts.json`)
- `dups.py`: exact cross-repo duplicate files after product-name normalisation
- `scripts.txt`: sha256 prefix of every `scripts/*` file per repo
- `rgtotals.txt`: plain rg totals for section 2

All 12 products pin baukit 0.18.2 (`baukit.toml` `version` and `baukit-core` in `backend/Cargo.toml`); templates are at 0.18.3. Version coherence is fine. The drift is everywhere else.

## 1. Conformance matrix

| repo | profile | backend cov floor | workspace lints (rust / clippy) | edition / rust-version | toolchain pin | clippy.toml | cargo-deny | eslint (flat, all) | tsconfig strict flags (files with flag) | prettier config | shellcheck / ruff | CLAUDE.md vs AGENTS.md | CLAUDE.md lines |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| eigenruhe | standard | 80 | unsafe_code=deny / all,dbg_macro,todo,unwrap_used warn | 2024 / 1.95 | none | no | no | mobile, content: strictTypeChecked+stylisticTypeChecked, react-hooks, rn-a11y, ui-tokens | strict 3, nUIA 2, eOPT 2 | mobile, content | no / no | copies, **differ** | 255 |
| hebkit | strict | 80 | same as template | **2021 / none** | none | no | no | web: strictTypeChecked with 4 rules off; **mobile: eslint-config-expo only, no typescript-eslint** | strict 3, nUIA 0, eOPT 0 | mobile | no / no | identical copies | 248 |
| leitbild | standard | 70 | template | 2024 / 1.95 | 1.99.0 (root) | no | no | web, mobile: strictTypeChecked | strict 2, nUIA 2, eOPT 2 | none (web has dep) | **yes** / no | identical copies | 86 |
| redemut | standard | 80 | unsafe_code=deny / **all only (no unwrap_used, todo, dbg_macro)** | 2024 / 1.95 | 1.99.0 (root) | no | no | root+web+mobile strictTypeChecked; web turns off no-unnecessary-condition | strict 3, nUIA 2, eOPT 2 | root | no / no | identical copies (turbo-injected block) | 136 |
| schlauzug | strict | 70 | template | 2024 / 1.95 | none | no | no | web, mobile, packages strictTypeChecked + eslint-plugin-react | all six flags in 3 files | packages | no / no | identical copies | 125 |
| tiefgang | strict | 80 | template | 2024 / 1.95 | none | no | no | mobile, extension, focus-core strictTypeChecked | strict 4, nUIA 3, eOPT 2 | none | no / no | identical copies | 196 |
| solo-leveling-system | strict | 80 | unsafe_code=deny, **warnings=deny** / **all=deny (no unwrap_used etc.)** | 2024 / **1.97.1** | 1.99.0 (backend/) | no | no | **root: eslint-config-expo + prettier only, `no-explicit-any: warn`**; mobile strictTypeChecked | strict 4, nUIA 1 | root `.prettierrc` | no / no | identical copies | 193 |
| lesezeichen | strict | 70 | template | 2024 / 1.95 | none | no | no | mobile strictTypeChecked; extension strictTypeChecked without stylistic | strict 3, nUIA 2 | none | no / no | identical copies | 70 |
| schluesselbund | strict | 70 | template | 2024 / 1.95 | none | no | no | web, mobile strictTypeChecked | strict 3, nUIA 2 | none | no / no | identical copies | 80 |
| kassensturz | strict | 80 | template | 2024 / 1.95 | none | no | **yes** (backend/deny.toml, run in gate) | mobile strictTypeChecked | all six, 1 file | none | no / no | identical copies | 73 |
| runtime-analyzer | strict | 70 | template in backend/; **agent/ workspace has no lints at all** | 2024 / 1.95 | none | no | **yes** | web strictTypeChecked | all six, 1 file | none | no / no | copies, **differ** (AGENTS 65 lines, stale template text) | 113 |
| architecture-health-platform | strict | 65 | template | 2024 / 1.95 | none | no | **yes** | web strictTypeChecked | all six, 1 file | web | no / no | **CLAUDE.md -> AGENTS.md symlink** | 45 |

nUIA = noUncheckedIndexedAccess, eOPT = exactOptionalPropertyTypes. Frontend coverage floors (jest/vitest `lines`/`statements`): 70 is the common value; outliers are leitbild/mobile 43/44, tiefgang/mobile 60, schlauzug/mobile none found, hebkit/mobile 77-90, redemut 78/81, SLS mobile 80.

Commands:
```sh
bash /home/patrick/.cache/baukit-orchestrator/quality-security/tmp/matrix.sh
for r in <repos>; do cmp -s $r/CLAUDE.md $r/AGENTS.md && echo same || echo DIFF; done
for r in <repos>; do grep -m1 '^edition' $r/backend/Cargo.toml; done
find <repo> -maxdepth 3 -name 'eslint.config.*' -not -path '*/node_modules/*'   # then grep -oE '(strict|recommended|stylistic)TypeChecked'
grep -cE 'shellcheck|ruff|cargo deny' <repo>/scripts/quality-gate.sh <repo>/Makefile
```
No repo has `clippy.toml`, `ruff.toml` or `.shellcheckrc` (`find -maxdepth 3`). Only leitbild mentions shellcheck in its gate or Makefile. The `shared` and `quality.affected` tables in `baukit.toml` differ per repo, which is expected.

## 2. Suppression and escape-hatch counts

Two views. The rg totals are matching **lines**, everything included. The split comes from `counts.py`, counts **occurrences**, skips `//` comment lines, and classifies as test: paths containing `/tests/`, `/test-support/`, `/benches/`, files ending `_test.rs`, `_tests.rs`, `tests.rs`, and every line after the first `#[cfg(test)]` in a file. TS test = `*.test.*`, `*.spec.*`, `__tests__/`, `e2e/`, `tests/`, `test/`. Exclusions in both: `node_modules target generated out dist coverage vendor .expo test-results playwright-report build`, plus `*.d.ts` and `*.gen.ts`.

rg totals (`rgtotals.txt`):
```sh
X=(-g '!**/node_modules/**' -g '!**/target/**' -g '!**/generated/**' -g '!**/out/**' -g '!**/dist/**' -g '!**/coverage/**' -g '!**/vendor/**' -g '!**/.expo/**' -g '!**/test-results/**' -g '!**/playwright-report/**' -g '!**/build/**')
rg -c '<pattern>' $r -t rust "${X[@]}" | awk -F: '{s+=$NF}END{print s}'
rg -c '<pattern>' $r -g '*.{ts,tsx,mts,cts}' -g '!*.d.ts' -g '!*.gen.ts' "${X[@]}" | awk -F: '{s+=$NF}END{print s}'
# patterns: '#\[allow\(' '\.unwrap\(\)' '\.expect\(' '\bpanic!\(' '\bunsafe\b' '\b(todo|unimplemented)!\('
#           '(:\s*any\b|\bas any\b)' 'as unknown as' '[\w\)\]]!\.' 'eslint-disable' '@ts-ignore' '@ts-expect-error' 'console\.log\('
```

| repo | allow | unwrap | expect | panic! | unsafe | todo!/unimpl | any | as unknown as | `!.` | eslint-disable | @ts-ignore | @ts-expect-error | console.log |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| eigenruhe | 28 | 0 | 1543 | 36 | 0 | 0 | 0 | 91 | 4 | 6 | 0 | 0 | 9 |
| hebkit | 29 | 46 | 2148 | 49 | 1 | 0 | 6 | 198 | 152 | 3 | 0 | 5 | 1 |
| leitbild | 5 | 0 | 112 | 31 | 0 | 0 | 0 | 24 | 0 | 5 | 0 | 0 | 0 |
| redemut | 1 | 911 | 285 | 32 | 0 | 0 | 0 | 99 | 0 | 16 | 0 | 2 | 1 |
| schlauzug | 4 | 0 | 3872 | 128 | 6 | 0 | 0 | 102 | 0 | 12 | 0 | 0 | 1 |
| tiefgang | 5 | 3 | 439 | 13 | 0 | 0 | 0 | 47 | 0 | 0 | 0 | 0 | 0 |
| solo-leveling-system | 4 | 1805 | 175 | 40 | 2 | 0 | 0 | 7 | 3 | 0 | 0 | 0 | 0 |
| lesezeichen | 0 | 0 | 1783 | 145 | 1 | 0 | 0 | 62 | 3 | 0 | 0 | 0 | 0 |
| schluesselbund | 0 | 0 | 884 | 18 | 2 | 0 | 0 | 25 | 0 | 0 | 0 | 0 | 0 |
| kassensturz | 0 | 0 | 1811 | 30 | 2 | 0 | 0 | 81 | 0 | 0 | 0 | 1 | 0 |
| runtime-analyzer | 1 | 726 | 3111 | 165 | 7 | 0 | 0 | 5 | 0 | 0 | 0 | 0 | 6 |
| architecture-health-platform | 2 | 1 | 6675 | 213 | 12 | 0 | 0 | 2 | 0 | 0 | 0 | 0 | 2 |

Split, non-test/test occurrences (`python3 tmp/counts.py`, head of `counts.out`):

| repo | allow | unwrap | expect | panic | unsafe | any | as unknown as | `!.` | eslint-disable | ts-expect-error | console.log |
|---|---|---|---|---|---|---|---|---|---|---|---|
| eigenruhe | 27/1 | 0/0 | 13/1536 | 1/35 | 0/0 | 0/0 | 14/77 | 0/10 | 3/3 | 0/0 | 9/0 |
| hebkit | 29/0 | 0/46 | 53/2106 | 7/42 | 0/1 | 0/6 | 77/121 | 34/131 | 3/0 | 5/0 | 0/1 |
| leitbild | 5/0 | 0/0 | 6/106 | 1/30 | 0/0 | 0/0 | 1/23 | 0/0 | 5/0 | 0/0 | 0/0 |
| redemut | 1/0 | 0/924 | 25/260 | 1/31 | 0/0 | 0/0 | 12/87 | 0/0 | 15/1 | 0/2 | 0/1 |
| schlauzug | 3/1 | 0/0 | 305/3671 | 7/121 | 0/6 | 0/0 | 2/100 | 0/0 | 12/0 | 0/0 | 0/1 |
| tiefgang | 5/0 | 0/3 | 30/414 | 1/12 | 0/0 | 0/0 | 31/16 | 0/0 | 0/0 | 0/0 | 0/0 |
| solo-leveling-system | 4/0 | 2/1855 | 58/117 | 0/40 | 0/2 | 0/0 | 1/6 | 0/4 | 0/0 | 0/0 | 0/0 |
| lesezeichen | 0/0 | 0/0 | 125/1685 | 2/143 | 0/1 | 0/0 | 36/26 | 0/3 | 0/0 | 0/0 | 0/0 |
| schluesselbund | 0/0 | 0/0 | 37/853 | 1/17 | 0/2 | 0/0 | 6/19 | 0/0 | 0/0 | 0/0 | 0/0 |
| kassensturz | 0/0 | 0/0 | 110/1735 | 1/29 | 0/2 | 0/0 | 41/42 | 0/0 | 0/0 | 1/0 | 0/0 |
| runtime-analyzer | 0/1 | 1/760 | 207/2952 | 1/164 | 1/6 | 0/0 | 3/2 | 0/0 | 0/0 | 0/0 | 6/0 |
| architecture-health-platform | 1/1 | 0/1 | 282/6468 | 2/211 | 1/11 | 0/0 | 1/1 | 0/0 | 0/0 | 0/0 | 0/2 |

Limits: a test-only module declared as `#[cfg(test)] mod foo;` in the parent and stored in `foo.rs` counts as non-test. Most of schlauzug's 305 non-test `expect` calls fall in that bucket (`room_service.rs:6988-6991` gates `crypt_benchmark` and `crypt_fixtures`). `unsafe` matches also hit `unsafe_code` in attributes and strings. `todo!`/`unimplemented!` and `@ts-ignore` are zero everywhere, and TS `any` is close to zero (the only 75 hits were in `runtime-analyzer/web/src/routeTree.gen.ts`, which is generated and excluded).

What the numbers say: `unwrap` is under control because 11 of 12 workspaces set `clippy::unwrap_used = "warn"` with `-D warnings`. Nothing restricts `expect`, so production code uses it as the escape hatch instead (see E3-08). On the TS side, `as unknown as` is the escape hatch of choice, because `strictTypeChecked` bans `any` but has no rule against double casts.

## 3. Size outliers

From `counts.out`. Lines include blank and comment lines.

| repo | Rust files / LOC | top Rust files (lines) | TS files / LOC | top TS/TSX files (lines) | Rust fns >100 lines (all / in non-test files) |
|---|---|---|---|---|---|
| eigenruhe | 292 / 114.8k | tests/postgres_integration.rs 4375, tests/coaching_api.rs 3973, tests/studio_api.rs 3799, eigenruhe-postgres/tests/coaching.rs 3665, eigenruhe-bin/src/animation_media.rs 2145 | 1154 / 155.5k | mobile/src/localization/de.ts 3461, en.ts 3345, features/today/today-screen.tsx 1396, animations/session-playback.test.ts 1248, player/player-shell.tsx 1221 | 131 / 34 |
| hebkit | 487 / 236.4k | hebkit-postgres/src/adapters/postgres/sync.rs 4985, .../nutrition.rs 4870, hebkit-bin/src/config.rs 4154, hebkit-domain/src/plan_generator.rs 3586, tests/trainer_authorization_matrix.rs 3391 | 1470 / 251.0k | mobile/src/sync/engine.test.ts 3585, app-tests/nutrition-screens.test.tsx 3247, db/sqlite/repositories.ts 2261, onboarding/plan-generator-contract.ts 2142, db/dexie/repositories.web.ts 2046 | 325 / 123 |
| leitbild | 82 / 37.9k | leitbild-bin/src/lib.rs 3019, tests/api_integration.rs 2777, tests/postgres_integration.rs 2177, leitbild-services/src/tests.rs 1780, tests/mcp_integration.rs 1522 | 412 / 56.7k | web/src/localization.tsx 2488, insights/InsightsPage.tsx 1311, pages/JournalEditorPages.tsx 1199, its test 1128, web/e2e/qa.config.ts 1119 | 42 / 10 |
| redemut | 96 / 46.5k | redemut-api/src/lib.rs 8492, redemut-content-compiler/src/lib.rs 2729, redemut-postgres/src/lib.rs 2229, redemut-services/src/lib.rs 1802, services/src/user_content.rs 1755 | 658 / 159.8k | packages/api-client/src/generated.ts 9528, localization/src/catalogs/web.ts 5091, web/src/content-builder-screens.tsx 3708, catalogs/mobile.ts 3349, web/src/milestone-d-screens.tsx 3150 | 41 / 20 |
| schlauzug | 318 / 201.5k | schlauzug-api/src/lib.rs 7421, schlauzug-services/src/room_service.rs 6992, schlauzug-bin/src/lib.rs 4488, services/src/run_service.rs 4473, domain/src/deck_expedition/mode.rs 4401 | 1278 / 244.2k | packages/game-content/src/crypt-descent.ts 4462, game-client/src/index.ts 3488, game-contracts/src/index.ts 3470, web/src/survivors-arena.tsx 2071, mobile/src/run-view.tsx 1973 | 172 / 91 |
| tiefgang | 173 / 51.6k | tiefgang-postgres/src/sync.rs 3441, tests/postgres_integration.rs 3318, tests/suite_integration.rs 1603, tiefgang-bin/src/config.rs 1424, services/src/integrations/mod.rs 1312 | 614 / 59.7k | mobile/src/db/sqlite/repositories.ts 1392, dexie/repositories.web.ts 850, db/repository-contract.ts 826, focus/session-screen-body.tsx 734, e2e/tests/quotas.spec.ts 732 | 61 / 29 |
| solo-leveling-system | 444 / 188.2k | api/tests/lifegraph_adaptive_quests.rs 2936, api/tests/notification_producers.rs 2763, services/src/quests.rs 2475, api/tests/lifegraph_trials.rs 2406, api/tests/lifegraph_guilds.rs 2301 | 709 / 107.4k | mobile/src/localization/messages-enterprise.ts 1698, oidc-auth.test.tsx 1299, attributes-screens.test.tsx 1001, journal-screens.test.tsx 977, messages-product.ts 961 | 195 / 75 |
| lesezeichen | 461 / 139.0k | integrations/src/pdf.rs 1995, tests/platform.rs 1795, postgres/tests/library.rs 1757, domain/src/limits.rs 1701, integrations/src/pdf/text.rs 1505 | 628 / 72.9k | mobile/src/features/reader/pdf/text-order.ts 2548, its test 938, local-data.test.tsx 782, pdf/reader-screen.tsx 765, selection-reader.tsx 762 | 124 / 46 |
| schluesselbund | 312 / 129.1k | tests/billing.rs 4628, tests/conversations.rs 3542, tests/invitations.rs 2729, tests/receivers.rs 2663, tests/revenuecat.rs 2597 | 467 / 50.6k | mobile/src/features/kompass/screens.test.tsx 865, web/.../kompass/screens.test.tsx 701, web/e2e/qa.config.ts 661, mobile/.../kompass/copy.ts 626, web/src/features/plans/plans.test.tsx 607 | 160 / 22 |
| kassensturz | 470 / 129.5k | tests/agent_tools.rs 2125, tests/platform_receiver.rs 1995, domain/src/recurring/model.rs 1862, tests/aggregator_regressions.rs 1845, tests/contracts_integration.rs 1744 | 1140 / 120.8k | mobile/src/domain/recurring/model.ts 1616, contract-tests/banking-regressions.test.ts 948, contracts.test.ts 900, holdings.test.ts 848, transactions/transaction-views.tsx 841 | 126 / 31 |
| runtime-analyzer | 761 / 270.5k | tests/api_sboms.rs 3580, tests/report_catalog.rs 3101, tests/evidence_integrity.rs 2872, tests/incidents.rs 2835, tests/api_products.rs 2492 | 546 / 55.1k | web/e2e/tests/incident-schema-fixture.ts 1730, governance-pages.test.tsx 1387, web/e2e/qa.config.ts 733, compliance/evidence.tsx 712, e2e/tests/risk-runtime.spec.ts 641 | 393 / 134 |
| architecture-health-platform | 979 / 403.3k | domain/src/limits.rs 4370, tests/maintenance_api.rs 3271, tests/architecture_intent_postgres.rs 3268, tests/g9_pr_governance.rs 3177, tests/inventory_postgres.rs 3074 | 452 / 66.4k | web/src/api.ts 1217, AutomationView.tsx 1156, FindingsView.test.tsx 1086, App.test.tsx 1069, ContextSettings.tsx 1040 | 479 / 140 |

The long-function counter is `fn_spans()` in `counts.py`. It first blanks out comments (`//`, `/* */`), string literals with escapes, raw strings (`r#"..."#`, any hash count) and char literals, keeping newlines. Then it finds every `fn name` with a regex and walks forward to the first `{` or `;`, skipping declarations that end in `;`. Finally it matches braces until depth returns to 0 and counts the lines from `fn` to the closing brace. Limits: nested fns and closures-in-fns are counted separately and also inside their parent. Macros that expand to fns (`#[tokio::test]` bodies are still plain fns, so those count) and `macro_rules!` bodies are not understood. A lifetime like `'a` next to a quote can rarely confuse char-literal blanking. "Non-test files" uses the path rule only, not `#[cfg(test)]`. A first naive version without string stripping reported 3740-line functions in `schluesselbund/backend/tests/billing.rs` because of JSON in raw strings, so treat any single value above ~1000 with suspicion. After the fix the largest are around 1000 lines.

## 4. Findings, ranked by impact

### E3-01 (high) Composition-root god functions, seeded by the template
`bin/api.rs::run` in 11 of 12 products is a single function of 206 to 666 lines that builds config, pools, rate limiters, suite, MCP, routers and the server. The template's own `run` is already 206 lines, so every product starts over the limit and grows from there.

- `architecture-health-platform/backend/crates/architecture-health-platform-bin/src/bin/api.rs:50` (666 lines)
- `schluesselbund/backend/crates/schluesselbund-bin/src/bin/api.rs:50` (544)
- `redemut/backend/crates/redemut-bin/src/application.rs:118` (512)
- `lesezeichen/backend/crates/lesezeichen-bin/src/bin/api.rs:46` (494), worker `run` at `lesezeichen-bin/src/bin/worker.rs:30` (579)
- `eigenruhe/backend/crates/eigenruhe-bin/src/bin/api.rs:67` (447)
- `kassensturz/backend/crates/kassensturz-bin/src/bin/api.rs:48` (444), `schlauzug/backend/crates/schlauzug-bin/src/bin/api.rs:76` (444)
- `baukit/templates/backend/backend/crates/__app__-bin/src/bin/api.rs:62` (206)

Command: the `fn_spans` loop over `glob('*/backend/crates/*-bin/src/bin/api.rs')` described in section 3. Across all repos, 755 functions in non-test files exceed 100 lines (sum of the last column in section 3).

Prevention: enable `clippy::too_many_lines` (pedantic, default threshold 100) in the template's `[workspace.lints.clippy]`, with a `clippy.toml` `too-many-lines-threshold` the template itself passes. Split the template `run` into `build_state`, `build_router` and `serve` first. A product override then needs a visible `#[expect(clippy::too_many_lines, reason = "...")]`.

### E3-02 (high) Hand-written `limits.json` validation grows into 350 to 900 line functions
Every product carries `backend/crates/*-domain/src/limits.rs` from the template (526 lines, `validate` at line 93). Products add one `match &self.section { Some(..) if .. => {}, _ => return Err(..) }` block per section. Sizes: AHP 4369 lines, lesezeichen 1700, schlauzug 1610, runtime-analyzer 1236, kassensturz 1185, schluesselbund 1172 (`wc -l */backend/crates/*-domain/src/limits.rs`).

- `architecture-health-platform/backend/crates/architecture-health-platform-domain/src/limits.rs:1035` `validate`, 903 lines
- `lesezeichen/backend/crates/lesezeichen-domain/src/limits.rs:436` (519)
- `runtime-analyzer/backend/crates/finops-domain/src/limits.rs:224` (408)
- `kassensturz/backend/crates/kassensturz-domain/src/limits.rs:338` (361), `schlauzug/backend/crates/schlauzug-domain/src/limits.rs:311` (353)

They also lean on `expect("embedded ... limits")` in non-test code (`architecture-health-platform-domain/src/limits.rs:37`, 30 occurrences in that file).

Prevention: custom check needed, plus a baukit feature. Move range, positive and ceiling checks into a declarative schema (`limits.schema.json` validated by one baukit-core routine, or a `#[derive(LimitsSection)]` with `#[limit(min, max)]` field attributes) so product code declares bounds instead of hand-coding them. Rule for the interim check: "`*-domain/src/limits.rs` must not contain a fn longer than 100 lines" (covered once E3-01's `too_many_lines` lands).

### E3-03 (high) Lint baseline drift: `expect` unrestricted, two repos off the template, one workspace with no lints
- The template sets `unsafe_code=deny` and clippy `all`, `dbg_macro`, `todo`, `unwrap_used` (`baukit/templates/backend/backend/Cargo.toml.jinja`). 9 repos match it.
- `redemut/backend/Cargo.toml:95-96` keeps only `all`, so `unwrap_used`, `todo` and `dbg_macro` are off. redemut has 911 `.unwrap()` lines, all in tests per the split.
- `solo-leveling-system/backend/Cargo.toml:93-97` replaces the template with `warnings = "deny"` and `all = "deny"`, which drops `unwrap_used`, `todo` and `dbg_macro` (restriction lints are not in `all`). 1805 unwrap lines, 2 in non-test code.
- `runtime-analyzer/agent/Cargo.toml` (a second Rust workspace, 4 crates: finops-agent, kube-inventory, finops-simulator, finops-scanner) has no `[workspace.lints]` and its crates have no `[lints]`. Check: `awk '/^\[lints\]/...' */backend/crates/*/Cargo.toml runtime-analyzer/agent/crates/*/Cargo.toml`, every other crate opts in.
- `hebkit/backend/Cargo.toml:16` is `edition = "2021"` with no `rust-version`; the other 11 use 2024 and 10 use `rust-version = "1.95"`. SLS uses 1.97.1. Toolchain pins differ too: leitbild, redemut and SLS pin 1.99.0, the other 9 have no pin.
- Nothing bans `expect`. Non-test `expect` occurrences: AHP 282, runtime-analyzer 207, lesezeichen 125, kassensturz 110. Examples: `architecture-health-platform/backend/crates/architecture-health-platform-api/src/privacy_map_routes.rs:37` (`expect("nonempty schema definition")` in an HTTP route module), `kassensturz/backend/crates/kassensturz-domain/src/recurring/model.rs:217`, `runtime-analyzer/backend/crates/finops-integrations/src/artifacts/conformance.rs:35` (61 in that file).

Prevention: ship the lint table as baukit-owned config that products cannot silently fork. Add `clippy::expect_used = "warn"` with `allow-expect-in-tests = true` and `allow-unwrap-in-tests = true` in a template `clippy.toml`. Add `clippy::allow_attributes_without_reason` and `clippy::allow_attributes` (forcing `#[expect(..., reason)]`) and `clippy::indexing_slicing` for domain crates. Custom check needed: "every `Cargo.toml` with `[workspace]` under the repo has `[workspace.lints]` that is a superset of the baukit template table, and every member crate has `[lints] workspace = true`". The same check should cover `edition`, `rust-version` and `rust-toolchain.toml`.

### E3-04 (high) Mobile ESLint in two repos skips typescript-eslint entirely
- `hebkit/mobile/eslint.config.js:7` is `...fixupConfigRules(expoConfig)` plus a11y and `react/jsx-no-literals`. No `strictTypeChecked`.
- `solo-leveling-system/eslint.config.mjs:7` is expo + prettier, with `"@typescript-eslint/no-explicit-any": "warn"` at line 45.
- `hebkit/web/eslint.config.js:62` turns `@typescript-eslint/no-non-null-assertion` off (one of 4 rules disabled there).
- Effect: hebkit has 152 non-null `!.` lines (rg), 34 non-test (`hebkit/mobile/src/features/today/today-weight-trend-card.tsx:133-134`), and 77 non-test `as unknown as` (`hebkit/mobile/src/db/dexie/sync-entities.web.ts`, 27 hits; `rg -c 'as unknown as' hebkit -g '*.{ts,tsx}' --glob '!node_modules'`). The other 10 repos have 0 to 4 non-null lines.
- Across all repos, `as unknown as` totals 743 lines (section 2), 225 occurrences of them in non-test code. kassensturz has 41, lesezeichen 36, tiefgang 31.

Prevention: publish `@baukit/eslint-config` (flat: `strictTypeChecked` + `stylisticTypeChecked` + react-hooks + a11y + ui-tokens) and make the gate fail if a product's `eslint.config.*` does not spread it. Custom check needed: "`as unknown as` outside test files fails unless the line carries `// baukit-allow-double-cast: <reason>`", or the community rule `@typescript-eslint/no-unsafe-type-assertion` (typescript-eslint >= 8.15) set to error.

### E3-05 (high) Template-owned scripts forked in place, 12 different `quality-gate.sh`
Every repo carries the same 20 template scripts, but most copies have diverged (`tmp/scripts.txt`; distinct sha256 count per name across 12 repos):

| script | repos | distinct hashes |
|---|---|---|
| quality-gate.sh | 12 | 12 |
| openapi.sh | 12 | 12 |
| reconcile_keycloak.py | 12 | 11 |
| lockfiles.sh | 12 | 11 |
| openapi-client.sh | 12 | 10 |
| pkce-login.py | 10 | 10 |
| test-keycloak-theme-patches.sh | 9 | 9 |
| preflight.sh | 12 | 6 |
| keycloak_policy.py | 12 | 5 |
| check-migrations-immutable.test.sh | 10 | 4 |
| gate.py | 12 | 1 |

Diff lines against the template (`diff baukit/templates/<path> <repo>/scripts/<name> | grep -c '^[<>]'`). For `reconcile_keycloak.py`: schlauzug 401, eigenruhe 235, SLS 66, kassensturz 54, four repos at 2. For `preflight.sh`: hebkit 112, leitbild 51. `quality-gate.sh` diffs range from 144 (schluesselbund) to 706 (eigenruhe), although part of that is Jinja blocks in the template.

`gate.py` stays at one hash because it is the library; the shell wrapper around it is what forks. 6 repos carry identical copies of `mobile/scripts/qa/android-sdk.sh` that no longer match the template (`tmp/dups.py`, "NOT-IN-TEMPLATE" rows), so a template fix will not reach them.

Prevention: custom check needed: "files listed in a baukit-owned manifest (`scripts/*.sh|*.py` from templates) must hash-match the rendered template for the pinned baukit version; product extensions go in `scripts/quality-gate.local.sh` hooks". `baukit doctor` is the natural place. Move the gate logic itself into the `baukit` CLI (`baukit gate`) so products stop carrying 383 to 639 line shell copies.

### E3-06 (medium) Suite composition copy-pasted into 8 products
`*-bin/src/suite.rs` defines the same `SuiteComposition { api, outbox, delivery, erasure, notifier }` and the same `new(pool, config, limiter)` wiring of `baukit_suite` adapters in 8 repos. The files are 83 to 118 non-blank lines with 0.62 to 0.71 line-sequence similarity (difflib) between products:

- `redemut/backend/crates/redemut-bin/src/suite.rs:25,32`
- `leitbild/backend/crates/leitbild-bin/src/suite.rs:30,50`
- `tiefgang/backend/crates/tiefgang-bin/src/suite.rs:26,34`
- `hebkit/backend/crates/hebkit-bin/src/suite.rs:25,34`
- `schlauzug/backend/crates/schlauzug-bin/src/suite.rs:22,30`
- `eigenruhe/.../eigenruhe-bin/src/suite.rs:37,45`, `kassensturz/.../kassensturz-bin/src/suite.rs:17,23`, `lesezeichen/.../lesezeichen-bin/src/suite.rs:25,34`

Command: `grep -n 'pub struct SuiteComposition\|impl SuiteComposition' */backend/crates/*-bin/src/suite.rs` and the difflib snippet (pairs per basename, `SequenceMatcher.ratio()` on whitespace-normalised non-blank lines). The only product-specific inputs are the four `*Suite{ErasureOwnerLookup,EventApplier,IdentitySource,ReplaySource}` impls and the catalog constants.

Prevention: baukit feature, not a lint. Add `baukit_suite::compose::PostgresSuite::builder(pool, settings, limiter).applier(..).identity(..).replay(..).erasure_lookup(..).build()`. Generated products then keep a 15-line suite.rs.

### E3-07 (medium) God modules: whole HTTP layer in one `lib.rs`
- `redemut/backend/crates/redemut-api/src/lib.rs` is 8492 lines with 163 `async fn` and 40 `.route(` calls; the router starts at `:3256`.
- `schlauzug/backend/crates/schlauzug-api/src/lib.rs` is 7421 lines with 76 `async fn` and 31 routes; the router is at `:838`.
- Same shape: `schlauzug/backend/crates/schlauzug-services/src/room_service.rs` (6992 lines, `apply_payload` at `:6477`, 350 lines), `schlauzug-bin/src/lib.rs` (4488), `leitbild/backend/crates/leitbild-bin/src/lib.rs` (3019), `hebkit/backend/crates/hebkit-bin/src/config.rs` (4154) and `hebkit-postgres/src/adapters/postgres/sync.rs` (4985).
- Frontend: `redemut/web/src/content-builder-screens.tsx` (3708) and `redemut/web/src/milestone-d-screens.tsx` (3150; a file named after a milestone), `schlauzug/packages/game-client/src/index.ts` (3488).
- `hebkit/backend/crates/hebkit-api/src/adapters/http/error.rs:131` `into_response` is 492 lines, one giant match mapping domain errors to HTTP.

Command: `grep -cE '^\s*(pub(\(crate\))? )?async fn' <file>`, `grep -cE '\.route\(' <file>`, and section 3.

Prevention: custom check needed: "non-test, non-generated source file over 2000 lines (Rust) or 1500 lines (TS/TSX) fails unless listed in `quality.size_exceptions` in `baukit.toml` with a reason". For TS, also set ESLint `max-lines` (error, 800, skipBlankLines and skipComments) and `max-lines-per-function` in the shared config.

### E3-08 (medium) Runtime SQL strings everywhere; compile-checked queries only in two repos
Only hebkit (972) and SLS (575) use `sqlx::query!`/`query_as!`. The other 10 repos use runtime `sqlx::query(...)` exclusively; one repo has a single macro use. `format!`-assembled SQL (identifier and predicate splicing, no value interpolation found in the sample):

| repo | `format!` SQL (non-tests/) | `sqlx::query(` runtime | `query!` macro |
|---|---|---|---|
| runtime-analyzer | 95 | 1379 | 0 |
| eigenruhe | 81 | 408 | 0 |
| SLS | 79 | 699 | 575 |
| tiefgang | 67 | 223 | 0 |
| AHP | 20 | 1045 | 0 |
| lesezeichen | 15 | 468 | 0 |
| kassensturz | 14 | 555 | 1 |
| schluesselbund | 3 | 656 | 0 |
| hebkit, leitbild, redemut, schlauzug | 0 | 82, 140, 99, 135 | 972, 0, 0, 0 |

Commands:
```sh
rg -U -c 'format!\(\s*r?#?"[^"]{0,40}\b(SELECT|INSERT INTO|UPDATE|DELETE FROM|WITH)\b' $r -g '*.rs' -g '!**/target/**' -g '!**/vendor/**' -g '!**/tests/**'
rg -c 'sqlx::query(_as|_scalar)?(::<[^>]*>)?\(' ...   # runtime
rg -c 'sqlx::query(_as|_scalar)?!\(' ...              # macro
```
References:
- `tiefgang/backend/crates/tiefgang-postgres/src/retention.rs:267` (`DELETE FROM {table} WHERE ctid IN (... {predicate} ...)`) and `tiefgang-postgres/src/guard.rs:739`
- `runtime-analyzer/backend/crates/finops-postgres/src/security.rs:407,430,456` (single-line 300+ char statements with `{table}`, `{predicate}`, `{key}`)
- `eigenruhe/backend/crates/eigenruhe-postgres/src/studio_tracking.rs:27` (`WHERE owner_id={owner}`, a column expression passed as `&str`) and `coaching_request.rs:112`
- `solo-leveling-system/backend/crates/solo-leveling-system-postgres/src/users.rs:435`
- `kassensturz/backend/crates/kassensturz-postgres/src/sync.rs:251`

None of these takes user input in the sample; the identifiers come from code. Every one of them is invisible to `cargo sqlx prepare`, though, and a typo in a table name surfaces only in Docker-gated tests.

Prevention: custom check needed: "`format!` whose literal starts with a SQL keyword fails unless every interpolated name is a `&'static str` from an allow-listed const table or `sqlx::QueryBuilder::push`" (a small rg/syn lint in baukit). Recommend `query!`/`query_as!` plus `.sqlx/` offline data as the template default; hebkit and SLS show it works at this scale. The security agent should look at `{owner}`/`{column}` callers.

### E3-09 (medium) CLAUDE.md / AGENTS.md: three different conventions and two live contradictions
- 9 repos keep two identical regular files. `hebkit/scripts/check-agent-instructions.sh:4` enforces exactly that ("AGENTS.md must be a regular file identical to CLAUDE.md") and rejects symlinks.
- `architecture-health-platform/CLAUDE.md` is a symlink to AGENTS.md, and `AGENTS.md:3` says "`CLAUDE.md` must remain a symlink to this file". That is the opposite of hebkit's check.
- `eigenruhe`: the copies disagree on behaviour. CLAUDE.md:200 says the suite catalog "accepts only `hebkit.sleep.recorded`", while AGENTS.md:200 says it "accepts no product event types" (`diff eigenruhe/CLAUDE.md eigenruhe/AGENTS.md`). The same split shows up at line 204.
- `runtime-analyzer`: AGENTS.md (65 lines) is still the generic template text ("generated with the strict quality profile") while CLAUDE.md (113 lines) carries the real runtime map. Codex-style agents read the stale one.
- `redemut/CLAUDE.md:129-135` contains a "# This is NOT the Turborepo you know" block that `turbo` re-injects when it detects an agent. It is the only file with em dashes (7) and the only one with a second H1.

Prevention: custom check needed: "AGENTS.md is the source; CLAUDE.md is a symlink to it (or a regular file byte-identical to it); `baukit doctor` fails otherwise". Pick one convention in the template (`baukit/templates/common/CLAUDE.md` is all that ships today). Set `"agentGuidance": false` in template `turbo.json`.

### E3-10 (medium) Fixed sleeps in tests
Totals per repo, Rust test files with `tokio::time::sleep|thread::sleep`, then Playwright `waitForTimeout`, then TS tests with `new Promise(... setTimeout`:

| repo | Rust | waitForTimeout | TS setTimeout |
|---|---|---|---|
| architecture-health-platform | 83 | 0 | 6 |
| lesezeichen | 58 | 0 | 4 |
| hebkit | 48 | 0 | 5 |
| schluesselbund | 47 | 0 | 6 |
| runtime-analyzer | 40 | 1 | 1 |
| kassensturz | 33 | 0 | 4 |
| schlauzug | 24 | 6 | 6 |
| SLS | 14 | 0 | 0 |
| eigenruhe | 10 | 1 | 6 |
| leitbild | 9 | 0 | 2 |
| redemut | 5 | 1 | 0 |
| tiefgang | 3 | 0 | 0 |

Commands:
```sh
rg -c '(tokio::time::sleep|thread::sleep|std::thread::sleep)\(' $r -g '*.rs' -g '!**/target/**' -g '!**/vendor/**' -g '**/tests/**' -g '*tests.rs'
rg -c 'waitForTimeout\(' $r -g '*.{ts,tsx}' -g '!**/node_modules/**'
rg -c 'new Promise\(\(?\w*\)? *=> *setTimeout' $r -g '*.{test,spec}.{ts,tsx}' -g '!**/node_modules/**'
```
Some of these are inside poll loops and fine. The fixed waits are the problem:
- `lesezeichen/backend/crates/lesezeichen-worker/tests/parse.rs:398` (`thread::sleep(1500ms)`)
- `redemut/backend/tests/it_ops.rs:74` (350 ms)
- `solo-leveling-system/backend/crates/solo-leveling-system-bin/tests/daily_quest_worker.rs:157,271`
- `architecture-health-platform/.../runtime-collector/tests/http_sink.rs:375`
- `runtime-analyzer/web/e2e/tests/sbom-upload.spec.ts:71` (`waitForTimeout(1200)`)
- `schlauzug/web/e2e/tests/deck-expedition-flow.spec.ts:127`

Prevention: ESLint `playwright/no-wait-for-timeout` (eslint-plugin-playwright) in the shared config. Custom check needed for Rust: "a `sleep(` call in a test file must sit inside a loop or carry `// baukit-allow-sleep: reason`"; baukit-test should offer a `wait_until(timeout, || cond)` helper. Also enable `clippy::disallowed_methods` for `std::thread::sleep` in a `clippy.toml` with test-file allowances.

### E3-11 (low) `#[allow]` without reason, concentrated in two repos
Lint kinds across all repos: `clippy::too_many_arguments` and `dead_code` dominate, with one each of `type_complexity` and `cast_possible_truncation` (`rg -o --no-filename '#\[allow\(clippy::[a-z_]+|#\[allow\([a-z_]+' <repo> -g '*.rs'`, vendor excluded). Non-test totals: hebkit 29, eigenruhe 27, all other repos 0 to 5.
- `eigenruhe/backend/crates/eigenruhe-api/src/connection_models.rs:9,16` and `api_token_models.rs:20` use `#[allow(dead_code)]` on DTOs.
- `eigenruhe/backend/crates/eigenruhe-postgres/src/program_adaptation.rs:31,649` and `eigenruhe-ports/src/lib.rs:624` use `too_many_arguments` on port methods, which points at missing parameter structs.

Prevention: `clippy::allow_attributes_without_reason = "warn"` and `clippy::allow_attributes = "warn"` (forces `#[expect(..., reason = "...")]`, which also fails once the suppression is stale).

### E3-12 (low) Framework type in a domain crate
`hebkit/backend/crates/hebkit-domain/Cargo.toml:21` has `utoipa.workspace = true` under `[dependencies]`, used in 15 domain source files (`rg -c utoipa hebkit/backend/crates/hebkit-domain/src | wc -l`). Every other domain crate has only dev-dependency tokio (`leitbild-domain/Cargo.toml:23`, `tiefgang-domain`). The rule exists in prose in several CLAUDE.md files ("Do not import Axum or SQLx into the domain crate", `lesezeichen/CLAUDE.md` Boundaries). Also `runtime-analyzer/backend/crates/finops-api/src/platform.rs:735-757` matches on `sqlx::Error` variants in the API crate (31 `sqlx::` references in finops-api; zero in the other 11 api crates).

Prevention: lift `schluesselbund/scripts/check-crate-deps.py` (112 lines, "Check the backend crate boundaries recorded in ADR 0001") into baukit as `baukit doctor --layers`, driven by `cargo metadata`. Rule: "`*-domain` and `*-ports` may not depend on axum, sqlx, utoipa, reqwest, tower, hyper or tokio (non-dev); `*-api` may not depend on sqlx".

### E3-13 (low) Product code shared between two products by copy
`tmp/dups.py` (sha1 of whitespace-normalised content after replacing product names, files of 40 lines or more, not matching any template file) finds 349 duplicate groups. Most are mobile QA scripts (see E3-05). The largest product-code cluster is the Blender animation pipeline, copied between `eigenruhe/animations/` and `hebkit/animations/`: about 30 identical Python files, for example `work_records.py` (344 lines), `authored_contracts.py`, `squat_equipment.py` and `prepare_surface.py`. `hebkit/mobile/src/tokens.ts` is identical in 5 repos. `scripts/reconcile_keycloak.py` is identical between runtime-analyzer and AHP but differs from the template.

Prevention: the animation pipeline should be one package (baukit or a shared animation repo); that is a product decision. The generic part is the same manifest-hash check as E3-05.

### E3-14 (low) Magic durations in production code
Non-test `Duration::from_secs|millis(<2+ digit literal>)` counts, which also include inline `#[cfg(test)]` modules: runtime-analyzer 50, hebkit 48, schlauzug 48, redemut 42, SLS 42, AHP 37, eigenruhe 29, schluesselbund 18, lesezeichen 17, kassensturz 15, tiefgang 13, leitbild 10. Command: `rg -c 'Duration::from_(secs|millis)\([0-9_]{2,}\)' $r -g '*.rs' -g '!**/target/**' -g '!**/vendor/**' -g '!**/tests/**' -g '!*tests.rs' -g '!*_test.rs'`.

Examples:
- `kassensturz/backend/crates/kassensturz-api/src/household.rs:93` and `kassensturz-mcp/src/quota.rs:22`, which repeat the same `Quota::new(.., Duration::from_secs(60), 0)`
- `hebkit/backend/crates/hebkit-bin/src/config.rs:1528` (`drain_timeout = 25s` hard-coded inside config)
- `hebkit/backend/crates/hebkit-api/src/adapters/clerk/mcp.rs:68`

Several CLAUDE.md files already say "user-visible limits come from `limits.json`".

Prevention: rate-limit and quota windows belong in `limits.json`/config. Custom check needed: "`Duration::from_*` with a literal outside a `const` item fails in `*-api`, `*-services`, `*-domain`". `clippy::default_numeric_fallback` does not help here.

## 5. Agent-instruction drift

Line counts range from 45 (AHP) to 255 (eigenruhe); the template is 78. Repos fall into three lineages:

- **Fitness lineage** (eigenruhe, hebkit, tiefgang): "What this is / Commands / Verify before you push / Architecture rules / Conventions / Log hygiene / Local environment".
- **Template lineage** (lesezeichen, schluesselbund, kassensturz, schlauzug, leitbild, runtime-analyzer): "Agent guide / Commands / Dependency and generated-file rules / Boundaries".
- **Own format** (SLS, redemut, AHP).

Keyword presence (`grep -ciE '<kw>' <repo>/CLAUDE.md`, order: eigenruhe hebkit leitbild redemut schlauzug tiefgang SLS lesezeichen schluesselbund kassensturz runtime-analyzer AHP):

| rule | counts | note |
|---|---|---|
| `--include-ignored` | 1 0 0 4 1 0 0 1 1 1 3 0 | missing in hebkit, leitbild, tiefgang, SLS, AHP, though the user memory requires Docker-gated tests in every verification. Should be shared. |
| Log hygiene / PII / secrets | 1 1 0 0 0 1 0 0 0 0 0 1 | only the fitness lineage has a "never log tokens, emails, request bodies" section (`hebkit/CLAUDE.md` Log hygiene). Should be shared (and enforced, see E2 security). |
| unwrap policy | 1 1 0 0 0 0 0 0 0 0 0 0 | only eigenruhe and hebkit mention it. Enforced by lints already, so drop from prose once E3-03 lands. |
| hexagonal / ports | 1 5 3 5 4 2 9 2 2 2 7 3 | present everywhere in different words. Should be one shared paragraph plus E3-12's check. |
| snapshot / insta | 11 10 1 3 4 5 4 2 3 3 0 3 | product-specific |
| Local ports section | leitbild, redemut, schlauzug, SLS | product-specific values, shared shape |
| "Never run migrations during API startup" | tiefgang, lesezeichen, schluesselbund, kassensturz (Boundaries) | shared rule, missing in the others |
| "limits come from limits.json, no numeric limits in components" | template lineage | shared rule |
| "semantic tokens from @baukit/ui-tokens, no raw colors" | template lineage | shared rule, and SLS already has `scripts/audit-theme.sh` enforcing it |

Contradictions:
1. Symlink vs identical copy: `architecture-health-platform/AGENTS.md:3` vs `hebkit/scripts/check-agent-instructions.sh:4` (E3-09).
2. eigenruhe CLAUDE.md:200 vs AGENTS.md:200 on which suite events the catalog accepts (E3-09).
3. runtime-analyzer: two different files under the two names (E3-09).
4. The baukit CLAUDE.md asks agents to mirror CI with `--include-ignored`; five product files list `cargo test` without it.

Recommendation: the template should own a shared "Baukit rules" block, inserted between markers such as `<!-- baukit:begin -->` / `<!-- baukit:end -->` and refreshed by `baukit upgrade`. Product text lives outside the markers. A check verifies that the block matches the pinned version.

## 6. Custom checks that should move into baukit

| check | where | what it does | proposal |
|---|---|---|---|
| `check-crate-deps.py` | `schluesselbund/scripts/` (112 lines) | enforces backend crate layering from ADR 0001 | `baukit doctor --layers` for all products (E3-12) |
| `check-agent-instructions.sh` | `hebkit/scripts/` | CLAUDE.md == AGENTS.md | baukit check after picking one convention (E3-09) |
| `check-egress.py` + `egress-allowlist.json` | `lesezeichen/scripts/` (122 lines) | outbound dependencies and source against reviewed exceptions | pairs with `baukit-egress`; generic version for every product |
| `audit-theme.sh` + `theme-allowlist.txt` | `solo-leveling-system/scripts/` (85 lines) | reports raw colors outside tokens, `--check` makes it a gate | enforce the "ui-tokens only" rule in every product |
| `audit-localization.mjs` + allowlist | `solo-leveling-system/scripts/` (322 lines) | catalog completeness | generic check on `@baukit/localization-core` catalogs |
| `check-limits-coverage.cjs` | `kassensturz/scripts/` (154 lines) | every `limits.json` key has tests per ingress (rest, sync_push, sqlite, dexie, mcp...) | generic limits-coverage check, pairs with E3-02 |
| `fuzz-statements.sh` + `backend/fuzz/` | `kassensturz` | cargo-fuzz targets for camt/mt940/csv parsers, time-boxed | baukit gate step `fuzz` that runs any `backend/fuzz` targets for N seconds when present |
| `check-expand-contract.sh` | `leitbild/scripts/` (59 lines) | pins migration hashes, enforces expand/contract | merge with template `check-migrations-immutable.sh` (2 hashes across 10 repos already) |
| `check-durable-tables.py` | `architecture-health-platform/scripts/` (95 lines) | rejects migration tables missing backup/restore evidence | generic, pairs with `baukit-erasure` / restore drills |
| `check-fixture-dates.py` | `architecture-health-platform/scripts/` (68 lines) | rejects fixture deadlines less than 90 days ahead (time-bomb tests) | generic, cheap, every product |
| `security-audit.py` | `kassensturz/scripts/` (71 lines) | pnpm audit with reviewed reachability exceptions that expire every 6 months | generic JS advisory gate; only 3 repos run cargo-deny today |
| `cargo deny` | kassensturz, runtime-analyzer, AHP | advisories and licenses | template `deny.toml` + gate step for all 12 |
| `observability-lint.py`, `check-observability.sh`, `check-observability-metrics.py`, `verify-observability.sh`, `check-alert-rules.sh` | eigenruhe, kassensturz, AHP, leitbild+AHP, schluesselbund | metric names, dashboard coverage, alert rules | five implementations of one idea; baukit already has `deploy/observability/lint/check-metric-names.py`, so ship one product-facing wrapper |
| `check-openapi-compatibility.py` | `solo-leveling-system/scripts/` | copy of the baukit script | delete once `openapi_compatibility` covers it |
| `check-tool-versions.sh` | `tiefgang/scripts/` (53 lines) | mise/rust/node pins agree | generic; would catch the rust-version/toolchain drift in E3-03 |
| `check-eslint-compat.mjs` | `schlauzug/scripts/` | ESLint config loads for web and mobile | absorbed by `@baukit/eslint-config` (E3-04) |
| `check-markdown-links.py`, `check-migrations-immutable.sh` | template, 10 to 11 repos | already shared | keep; `redemut` and `leitbild` do not wire migrations-immutable into the gate (grep count 0) |

Not found anywhere: shellcheck (leitbild only), ruff, typos, cargo-machete, `clippy.toml`. All are cheap to add once in the template gate.
