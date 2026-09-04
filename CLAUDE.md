# CLAUDE.md

Guidance for coding agents working in this repository.

## Project

**baukit** (from German *Baukasten*, a modular construction kit) is a
private-first application foundation: shared runtime, configuration, HTTP,
operations, telemetry, testing, and frontend contracts reused across
independently buildable Rust-backed products. It is a library/toolkit repo —
not a framework and not a product monorepo. Keep crates small, composable, and
free of product-specific logic.

## Layout

```text
rust/crates/        baukit-runtime, -config, -http, -ops, -telemetry, -openapi, -test
typescript/packages/ a11y-core, analytics-core, analytics-posthog-{web,native},
                     api-runtime, auth-{native,node,web},
                     data-contracts{,-dexie,-expo-sqlite}, localization-core,
                     preferences-core, ui-tokens
cli/                `baukit` CLI (scaffolds products from templates/)
templates/          project templates consumed by the CLI
deploy/             Helm chart + observability (dashboards, alerts, recording rules)
agent-skills/       installable agent skills (make install-skills TARGET=<dir>)
examples/           runnable examples
```

Three separate workspaces: `rust/`, `cli/` (Rust), and `typescript/`
(corepack-pinned pnpm + Turborepo, Node version from `typescript/.nvmrc`).
Always pass the right `--manifest-path` / `--dir`; there is no root workspace.

## Verification — mirror CI before finishing

CI (`.github/workflows/ci.yml`) is strict and the pipeline must stay green.
Before declaring any task done, run the same checks CI runs, locally.

`make ci` covers most of it (fmt, clippy `-D warnings`, tests, and cargo check
for `rust/` and `cli/`, plus pnpm build/format/lint/test/check for
`typescript/`). CI additionally runs these — execute the ones relevant to your
change:

| CI job | Run locally |
|---|---|
| Release version coherence | `scripts/check-version-coherence.py` (always after touching any version) |
| Cargo deny | `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check advisories licenses` (after dependency changes) |
| MSRV (Rust 1.95) | `cargo +1.95 check --manifest-path rust/Cargo.toml --workspace --all-targets` (after using new language/std features) |
| Observability metric names | `python3 deploy/observability/lint/check-metric-names.py` (after touching deploy/observability or metric names) |
| Dexie real-browser conformance | `make ts-browser-test` |
| Generated mobile Android compile | `make native-android-gate` (after touching mobile templates, CLI generation, or their native dependencies) |
| Real Expo SQLite on Android | `make expo-sqlite-conformance` (after touching data contracts or the Expo SQLite adapter) |
| Generated fixture | see below (after touching cli/, templates/, or public APIs the templates use) |

Docker is available locally — never skip Docker-gated (`#[ignore]`) tests; run
`cargo test --manifest-path rust/Cargo.toml -- --include-ignored` when your
change touches anything they cover.

### Golden snapshot trees

`cli/tests/snapshots/*.tree` pins a SHA-256 per generated file. Any template or
generator change makes those tests fail. Regenerate them, then read the diff and
confirm every changed path is one you meant to touch:

```bash
cargo run --manifest-path cli/Cargo.toml --example bless_snapshots
git diff --stat cli/tests/snapshots/
```

### Generated-fixture check

CI scaffolds a product with the CLI and verifies the output builds clean. If
you changed `cli/`, `templates/`, or a public API that generated code consumes:

```bash
cargo build --manifest-path cli/Cargo.toml --bin baukit
cli/target/debug/baukit new fixture --backend --mobile --web --dir .generated-fixture --baukit-path rust
cargo fmt   --manifest-path .generated-fixture/fixture/backend/Cargo.toml --all --check
cargo clippy --manifest-path .generated-fixture/fixture/backend/Cargo.toml --all-targets -- -D warnings
cargo test  --manifest-path .generated-fixture/fixture/backend/Cargo.toml
cargo test  --manifest-path .generated-fixture/fixture/backend/Cargo.toml -p fixture-bin --test openapi_drift
# web:    cd .generated-fixture/fixture/web    && pnpm install && pnpm build && pnpm lint && pnpm test
# mobile: cd .generated-fixture/fixture/mobile && pnpm install && pnpm exec tsc --noEmit && pnpm lint && pnpm test
# MCP:    `make mcp-fixture-gate` generates `--backend --mcp`, checks the backend,
#         then builds, lints, typechecks, tests, and runs both MCP drift checks.
```

(For web/mobile flavors, first build the local TS deps:
`corepack pnpm --dir typescript install --frozen-lockfile && corepack pnpm --dir typescript --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/api-runtime --filter @baukit/auth-native --filter @baukit/data-contracts --filter @baukit/ui-tokens run build`.)

## Conventions

- Rust: rustfmt + clippy clean (`-D warnings`) in `rust/`, `cli/`, **and all
  generated fixture output** — template code is linted like committed code.
- TypeScript: strict, prettier `format:check`, eslint, vitest via Turborepo;
  use `corepack pnpm`, never a globally installed pnpm.
- Versions must stay coherent across crates, packages, templates, and the
  chart — `scripts/check-version-coherence.py` is the arbiter.
- Metric names must pass `deploy/observability/lint/check-metric-names.py`;
  dashboards/alerts/recording rules reference those names, keep them in sync.
- Templates in `templates/` generate downstream products: any template change
  must keep the generated fixture green in all three CI flavors
  (backend, web, combined).
