# Strict quality profile

`baukit new --quality strict` adds a blocking CI job and `scripts/quality-gate.sh`. The script runs the same checks locally in the same order. The standard profile remains the default.

Every strict project also gets `scripts/check-markdown-links.py`. The strict gate runs its standard-library test suite, then checks committed Markdown under `README.md`, `CLAUDE.md`, `AGENTS.md`, and `docs/`. Relative links resolve from the source file. Repository-absolute links resolve from the product root. External URLs, query strings, and fragments do not trigger network requests. The checker reports the source file, line, and missing target. It does not validate anchors.

The generator reads capabilities before it renders the workflow. A backend gets coverage, MSRV, migration, OpenAPI, and production-image checks. A web app gets unit coverage plus the complete Chromium and WebKit Playwright suite, including geometry and console-warning specs. A mobile app gets Expo Doctor, lint, type checks, Jest coverage, an iOS JavaScript bundle, and Android `assembleDebug`. Missing capabilities do not leave placeholder jobs.

## Manifest settings

The root manifest owns the strict thresholds and product declarations:

```toml
[quality]
profile = "strict"
backend_coverage_lines = 70
critical_paths = []
webkit_repeats = 3
full_stack_e2e = false

[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts"]
```

`critical_paths` contains Playwright spec paths relative to `web/`, such as `e2e/tests/checkout.spec.ts`. The strict runner executes those specs on desktop and mobile WebKit projects with `--repeat-each`. Keep this list short and reserve it for flows where intermittent WebKit failures would block a release.

`full_stack_e2e` stays false until the product has an external-service harness. When enabled, the repository must provide `scripts/full-stack-e2e.sh`. That product-owned script starts services, loads deterministic fixtures, runs the tests, and cleans up.

List each committed OpenAPI TypeScript declaration in `openapi.consumers`. `scripts/openapi-client.sh` regenerates the entire list. The strict gate fails when a listed file is uncommitted or changes after regeneration.

## Local use

Install the tools required by the enabled capabilities, then run:

```sh
sh scripts/quality-gate.sh
```

For migration checks, set `BAUKIT_BASE_REVISION` to the pull request base commit. Without it, the script uses the merge base with `origin/main`, then `HEAD^`, then the first commit.

The backend coverage gate uses `cargo llvm-cov nextest --run-ignored all`. Docker must be running because generated PostgreSQL tests use `#[ignore]`. Coverage HTML and LCOV files are written under `backend/target/llvm-cov/`, and CI uploads both.

Observability checks run only when `scripts/observability-lint.py` exists. A production image builds only when `backend/Dockerfile` exists. The generated web app sets `capabilities.pwa = false`. If a product adds a PWA and changes that value to true, its web package must provide `build:sw:check`; the strict runner calls it in both local and CI runs.

## Migration

Existing strict products can copy `scripts/check-markdown-links.py` and its test from the current template. Add both commands to `scripts/quality-gate.sh` before capability-specific build checks:

```sh
python3 scripts/check-markdown-links.test.py
python3 scripts/check-markdown-links.py README.md CLAUDE.md AGENTS.md docs
```

Pass different repository-relative roots if the product keeps Markdown elsewhere. Commit the files before checking them because the script uses `git ls-files` to keep local and CI input identical.
