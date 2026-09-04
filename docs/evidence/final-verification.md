# Final verification

## 1. Summary

PostgreSQL 18 now mounts its volume at `/var/lib/postgresql`. The CLI renderer and snapshot tool ignore `__pycache__` directories and `.pyc` files, and a unit test covers binary cache files beside templates. All eight snapshot trees were regenerated and inspected.

Generated OIDC MCP code needed one Prettier correction. The strict quality gate also failed in a fresh project because it required Git history. It now checks files on disk before the first commit, compares OpenAPI consumers with temporary copies, and skips only the history-dependent migration comparison. Repositories with history retain the committed-file and migration checks. This was extra work found by the requested verification, not a plan deviation. The worker fixture was not run because `.github/workflows/ci.yml` has no worker matrix entry.

| Evidence field | Record |
|---|---|
| Source product files | `/home/patrick/projects/redemut/scripts/check-doc-links.mjs` and its workflow command are the related duplicate recorded for plan item 15. The PostgreSQL and Python-cache failures came from fresh generated fixtures. |
| Observed failure or repeated glue | PostgreSQL 18 rejected the old data mount. Python bytecode broke template embedding. Fresh strict projects had no Git history. OIDC MCP output failed Prettier. |
| Baukit owner | `templates/backend`, `templates/common/__strict__`, `templates/mcp`, and the CLI renderer and snapshot tool. |
| Public types and errors | No library types changed. Generated scripts retain exit 1 for drift and exit 2 for invalid setup. |
| Product-owned inputs | `baukit.toml`, selected capabilities, OpenAPI consumer paths, Markdown roots, and `BAUKIT_BASE_REVISION` after Git history exists. |
| Concurrency, failure, privacy, cleanup | Rendering skips cache artifacts deterministically. Strict consumer checks use isolated temporary copies. Docker proofs removed containers and volumes. No tokens or payloads were logged. |
| Supported runtimes | Rust 1.95 and stable, Python 3, Node with pinned pnpm, Docker Compose, Chromium, WebKit, and Android API 36. |

## 2. Files added or changed

- `cli/examples/bless_snapshots.rs`
- `cli/src/lib.rs`
- `cli/tests/snapshots/auth.tree`
- `cli/tests/snapshots/backend.tree`
- `cli/tests/snapshots/combined.tree`
- `cli/tests/snapshots/mcp.tree`
- `cli/tests/snapshots/mobile.tree`
- `cli/tests/snapshots/strict.tree`
- `cli/tests/snapshots/web.tree`
- `cli/tests/snapshots/worker.tree`
- `docs/evidence/final-verification.md`
- `templates/backend/compose.yaml`
- `templates/common/CHANGELOG.md`
- `templates/common/CLAUDE.md`
- `templates/common/__strict__/scripts/check-markdown-links.py`
- `templates/common/__strict__/scripts/check-markdown-links.test.py`
- `templates/common/__strict__/scripts/quality-gate.sh`
- `templates/mcp/mcp/src/auth.ts.jinja`

## 3. Verification

| Command | Result |
|---|---|
| `docker compose up -d --wait postgres` in a fresh `--backend` fixture | Pass. PostgreSQL 18 became healthy on a new volume. |
| `docker compose down -v` in that fixture | Pass. Container, network, and volume removed. |
| `cargo fmt --manifest-path cli/Cargo.toml --all --check` | Pass. |
| `cargo clippy --manifest-path cli/Cargo.toml --all-targets -- -D warnings` | Pass. |
| `cargo test --manifest-path cli/Cargo.toml` | Final pass: 10 unit and 37 generator tests, 47 total. |
| `PYTHONDONTWRITEBYTECODE=1 python3 templates/common/__strict__/scripts/check-markdown-links.test.py` | Pass: 4 tests. |
| `find templates -type d -name __pycache__ -print` and `find templates -type f -name '*.pyc' -print` | Pass: no results. No cache artifacts remained to remove. |
| `cargo run --example bless_snapshots` from `cli/` | Pass: all 8 trees blessed. |
| `git diff --stat tests/snapshots/` from `cli/` | Pass: 8 files, 19 insertions and 19 deletions. Changes are compose hashes, generated changelog hashes, and strict documentation and script hashes. |
| `make ci` | Final pass. Rust and CLI fmt, clippy, tests, and checks passed. CLI passed 47 tests. TypeScript passed all 18-package stages and 26 browser tests. Platform validation passed 14 bases, 14 charts, and 2 overlays. |
| `cargo test --manifest-path rust/Cargo.toml --workspace -- --include-ignored` | Pass, 0 failures and 0 ignored at completion. Docker suites ran for jobs on PostgreSQL, rate limiting on Redis and Sentinel, sync on PostgreSQL, and `baukit-test`. |
| `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check advisories licenses` | Pass: advisories and licenses. It warned that locked `chacha20 0.10.1` is yanked. |
| `cargo +1.95 check --manifest-path rust/Cargo.toml --workspace --all-targets` | Pass. |
| `python3 deploy/observability/lint/check-metric-names.py` | Pass: 1 dashboard, 2 rule files, 9 recording rules. |
| `scripts/check-version-coherence.py` | Pass: version 0.2.1 across 16 crates, CLI, 18 packages, and 2 charts. |
| `make ts-browser-test` | Pass: 2 files and 26 tests on Chromium and WebKit. |
| Workflow TypeScript dependency install and filtered build | Pass for the 7 local packages required by generated fixtures. |

Every generated backend ran fmt, clippy with `-D warnings`, normal tests, `cargo test ... -- --include-ignored`, and the explicit OpenAPI drift test. Every included PostgreSQL test ran through Docker. Web fixtures ran install, build, lint, and test. Mobile fixtures ran install, `pnpm exec tsc --noEmit`, lint, and test. MCP fixtures ran install, build, typecheck, lint, test, `openapi:check`, and `docs:check`.

| Generated command | Result |
|---|---|
| `baukit new fixture --backend --dir "$fixture_parent" --baukit-path rust` | Pass: backend 9 normal and 10 with ignored tests included; drift 1. |
| `baukit new fixture --backend --web --dir "$fixture_parent" --baukit-path rust` | Pass: backend as above; web 8 files and 17 tests, worker script 2 tests. |
| `baukit new fixture --backend --mobile --dir "$fixture_parent" --baukit-path rust` | Pass: backend as above; mobile 10 suites and 31 tests. |
| `baukit new fixture --backend --mobile --web --dir "$fixture_parent" --baukit-path rust` | Pass: backend as above; web 17 plus 2 tests; mobile 31 tests. |
| `baukit new fixture --backend --mobile --web --auth oidc --dir "$fixture_parent" --baukit-path rust` | Pass: backend 12 normal and 13 with ignored tests included; web 22 plus 2 tests; mobile 40 tests. `make dev` started healthy Keycloak and printed `Keycloak development realm reconciled.` `docker compose down -v` cleaned it up. |
| `baukit new fixture --backend --mobile --web --auth oidc --mcp --dir "$fixture_parent" --baukit-path rust` | Final pass: OIDC counts above; MCP 2 files and 9 tests; both drift checks passed. |
| `baukit new fixture --backend --mcp --dir "$fixture_parent" --baukit-path rust` | Pass: backend 9 normal and 10 with ignored tests included; MCP 2 files and 9 tests; both drift checks passed. |
| `baukit new fixture --backend --web --quality strict --dir "$fixture_parent" --baukit-path rust` followed by `sh scripts/quality-gate.sh` | Final pass on a fresh project: backend 9 normal and 10 with ignored tests included; web 17 plus 2 tests. Strict gate ran 10 Rust tests including PostgreSQL, coverage, MSRV, migration self-tests, OpenAPI checks, a Docker image build, 17 covered web tests, and 83 browser tests with 9 skips. |
| `make mcp-fixture-gate` | Pass: backend checks and 9 MCP tests in 2 files. |
| `make native-android-gate` | Pass: Android debug APK, 489 Gradle tasks. |
| `make expo-sqlite-conformance` | Pass: Android emulator reported `BAUKIT_SQLITE_CONFORMANCE_PASS {"passed":23}`. |
| `corepack pnpm --dir typescript exec changeset status --verbose` | Command passed, but computed major 1.0.0 for all 18 fixed-group packages instead of minor. |
| `git diff --check` | Pass. |

Failures and fixes found during verification:

- CLI tests first passed 31 and failed 6 after the compose change, then passed 29 and failed 8 after the strict fix. Both were expected stale snapshots. Re-blessing from `cli/` restored all 47 tests.
- Two OIDC MCP lint runs failed on `src/auth.ts`. The template now uses Prettier's one-line client ID expression and multiline error write. A fresh full fixture passed.
- The first strict gate failed because `git ls-files` had no repository. The bootstrap behavior described above fixed it, and two fresh full gates passed.
- `cargo run --manifest-path cli/Cargo.toml --example bless_snapshots` from the repository root failed because the example expects `tests/snapshots` relative to `cli/`. Running `cargo run --example bless_snapshots` from `cli/` passed.
- One focused Python test invocation from `cli/` used a repository-root path and failed. The same exact command from the repository root passed 4 tests.
- Changesets still reports major. Every authored changeset requests minor. `@changesets/assemble-release-plan` 6.0.10 promotes the peer dependents of `@baukit/analytics-core` and `@baukit/data-contracts` because `onlyUpdatePeerDependentsWhenOutOfRange` defaults to false. The fixed group in `typescript/.changeset/config.json` then propagates major to all 18 packages. Bump types and config were left unchanged as requested.

## 4. Failures observed in other agents' areas

None. No other agent was editing the tree during this pass.

## 5. Leftovers and open questions

Changesets' computed major release is the only unresolved release blocker. The orchestrator must decide whether the fixed group and peer-dependent policy should change. The yanked `chacha20 0.10.1` lockfile warning is not a deny failure, but a later lockfile refresh should move it to 0.10.2.

## 6. Product adoption note

Redemut can delete `/home/patrick/projects/redemut/scripts/check-doc-links.mjs` and its dedicated workflow command when it adopts the generated strict gate. The final verification fixes do not make other product files deletable.
