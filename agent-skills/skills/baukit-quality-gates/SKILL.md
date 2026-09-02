---
name: baukit-quality-gates
description: Select, configure, run, and diagnose a generated Baukit product's standard or strict quality profile. Use when choosing `--quality strict`, editing the manifest's quality declarations, reproducing generated CI, or interpreting a quality-gate failure.
---

# Work with Baukit quality gates

Read `baukit.toml`, `CLAUDE.md`, `scripts/quality-gate.sh`, and `.github/workflows/ci.yml` before changing a gate. The manifest owns thresholds, OpenAPI consumers, critical paths, and the full-stack E2E opt-in. The generated guide states which commands apply to the selected capabilities.

## Choose a profile

Use the standard profile for prototypes or products that do not yet have the required native, browser, Docker, and Rust coverage tooling. It keeps the ordinary generated checks.

Use `baukit new ... --quality strict` when every enabled capability must pass its production checks on each change. Strict is a blocking profile. Do not turn a failed check into a skip because a local tool is missing.

Keep `quality.full_stack_e2e = false` until the product has a deterministic `scripts/full-stack-e2e.sh`. External services, credentials, cleanup, and test data belong to that product-owned script.

## Run before pushing

From the product root:

```sh
baukit doctor
sh scripts/quality-gate.sh
```

Set `BAUKIT_BASE_REVISION` to the pull request base commit when the local branch cannot resolve `origin/main`. The runner follows CI order and stops at the first failure.

## Read failures

- Coverage failures name the measured line percentage and threshold from `quality.backend_coverage_lines`. Add tests for missed behavior or make a deliberate manifest change. Do not exclude product modules to move the number.
- Migration failures mean a file present at the base revision was modified, deleted, or renamed. Restore it and add a forward migration.
- OpenAPI failures mean the schema or a path in `openapi.consumers` changed after regeneration. Regenerate all consumers, inspect the contract change, and commit the intended files.
- Browser failures include Playwright traces and reports. Reproduce the named project. Geometry and console-warning failures are product bugs unless the product-owned QA configuration documents a narrow exception.
- Critical-path failures use the spec files in `quality.critical_paths` and repeat them on both WebKit projects. Fix intermittent state, timing, or cleanup instead of lowering `quality.webkit_repeats` without a product decision.
- Native failures occur after a clean Expo prebuild. Treat missing config-plugin output, iOS bundle errors, and Android compiler errors as dependency or app-configuration failures.
- Observability failures mean a dashboard, rule, or query references a metric outside the Baukit and product vocabularies. Register the product metric or correct the query.

When a local machine lacks a required tool, use the generated CI job as the authoritative run and report the missing local prerequisite. Keep the gate blocking.
