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
rust/crates/        baukit-auth, -config, -core, -credential-vault, -data-subject, -egress, -erasure, -events,
                     -http, -integrations, -jobs, -mcp, -openapi, -ops, -push, -ratelimit,
                     -runtime, -suite, -sync, -telemetry, -test
typescript/packages/ a11y-core, analytics-core, analytics-posthog-{web,native},
                     api-runtime, auth-{native,node,web},
                     data-contracts{,-dexie,-expo-sqlite}, events,
                     integrations-client, localization-core, navigation,
                     notifications-{core,expo}, preferences-core, pwa-web,
                     suite-client, sync-client, ui-tokens
cli/                `baukit` CLI (scaffolds products from templates/)
templates/          project templates consumed by the CLI
deploy/             Helm chart, observability (dashboards, alerts, recording rules),
                    media-grants (njs edge verifier)
fixtures/           shared cross-language test vectors
agent-skills/       installable agent skills (make install-skills TARGET=<dir>)
examples/           runnable examples
```

Three separate workspaces: `rust/`, `cli/` (Rust), and `typescript/`
(corepack-pinned pnpm + Turborepo, Node version from `typescript/.nvmrc`).
Always pass the right `--manifest-path` / `--dir`; there is no root workspace.

## Verification — mirror CI before finishing

CI (`.github/workflows/ci.yml`) is strict and the pipeline must stay green.
Before declaring any task done, run the same checks CI runs, locally.

`sh scripts/ci-gate.sh` (or `make ci-gate`) runs every CI job's checks in parallel lanes,
and each CI job calls one of its steps with `scripts/ci-gate.sh --step NAME`.
`--list-steps` prints the step table, `--affected BASE` runs only the lanes that
`scripts/ci-gate.toml` maps to changed paths, and `--plan` prints the selection
without running it. Timings and lane logs land in `.ci-gate/`.

`make ci` covers most of it (fmt, clippy `-D warnings`, tests, and cargo check
for `rust/` and `cli/`, plus pnpm build/format/lint/test/check for
`typescript/`, the `scripts/` tests, and both media-grant vector suites; the
njs suite needs Docker). CI additionally runs these — execute the ones relevant to your
change:

| CI job | Run locally |
|---|---|
| Release version coherence | `scripts/check-version-coherence.py` (always after touching any version) |
| Cargo deny | `cargo deny --manifest-path rust/Cargo.toml --config rust/deny.toml check advisories licenses` (after dependency changes) |
| MSRV (Rust 1.95) | `cargo +1.95 check --manifest-path rust/Cargo.toml --workspace --all-targets` (after using new language/std features) |
| Observability metric names | `python3 deploy/observability/lint/check-metric-names.py` (after touching deploy/observability or metric names) |
| Dexie real-browser conformance | `make ts-browser-test` |
| Generated mobile Android compile | `make native-android-gate` (after touching mobile templates, CLI generation, or their native dependencies) |
| Real Expo SQLite on Android | `make expo-sqlite-conformance` (after touching data contracts, the Expo SQLite adapter, or `scripts/expo-android-conformance.sh`) |
| Hermes vectors and expo-notifications on Android | `make expo-notifications-conformance` (after touching localization-core, the notifications packages, their fixtures, or `scripts/expo-android-conformance.sh`) |
| Generated fixture | see below (after touching cli/, templates/, or public APIs the templates use) |

Docker is available locally — never skip Docker-gated (`#[ignore]`) tests; run
`cargo test --manifest-path rust/Cargo.toml -- --include-ignored` when your
change touches anything they cover.

On a new Linux workstation, run `make ts-browser-deps` once to install the
Playwright browsers and their OS packages. `make ts-browser-test` installs only
the browser binaries, so routine checks do not run the system package manager.
It runs each browser suite through `scripts/with-loopback-network.sh`, which gives
the Vite server and browsers a private network namespace with only loopback. A
Docker network created or removed by another gate on the host otherwise fires
Chromium's network-change handling and aborts module requests, so a tester
iframe never reports ready. The script needs unprivileged user namespaces.

### Golden snapshot trees

`cli/tests/snapshots/*.tree` pins a SHA-256 per generated file. Any template or
generator change makes those tests fail. Regenerate them, then read the diff and
confirm every changed path is one you meant to touch:

```bash
cargo run --manifest-path cli/Cargo.toml --example bless_snapshots
git diff --stat cli/tests/snapshots/
```

### Generated-fixture check

`make generated-fixture-gate` runs all fixture flavors from CI: backend, web,
mobile, mobile-pwa, mobile-pwa-auth, combined, mcp, auth, clerk, workos, platform-standalone and
platform. `make platform-fixture-gate` checks standalone and OIDC platform
backends with web clients, including signed-event PostgreSQL tests and the daily
worker purge. Use `FIXTURE_FLAVORS="backend platform"` with the full target for a
subset.

CI scaffolds a product with the CLI and verifies the output builds clean. If
you changed `cli/`, `templates/`, or a public API that generated code consumes:

```bash
export CI=true CARGO_BUILD_JOBS=6 VITEST_MAX_WORKERS=4
fixture_name=long-product-name-fixture
cargo build --manifest-path cli/Cargo.toml --bin baukit
cli/target/debug/baukit new "$fixture_name" --backend --mobile --web --mcp --auth oidc --dir .generated-fixture --baukit-path rust
cargo fmt --manifest-path ".generated-fixture/$fixture_name/backend/Cargo.toml" --all --check
cargo clippy --manifest-path ".generated-fixture/$fixture_name/backend/Cargo.toml" --all-targets -- -D warnings
cargo test --manifest-path ".generated-fixture/$fixture_name/backend/Cargo.toml" -- --include-ignored
cargo test --manifest-path ".generated-fixture/$fixture_name/backend/Cargo.toml" -p "$fixture_name-bin" --test openapi_drift
# web:    cd ".generated-fixture/$fixture_name/web" && corepack pnpm install && corepack pnpm build && corepack pnpm lint && corepack pnpm test && corepack pnpm run test:coverage
# mobile: cd ".generated-fixture/$fixture_name/mobile" && corepack pnpm install && corepack pnpm exec tsc --noEmit && corepack pnpm lint && corepack pnpm run test:coverage
# MCP:    `make mcp-fixture-gate` generates this combined OIDC fixture, checks
#         the backend, web and mobile, runs tool_drift, and calls MCP over HTTP
#         with a real Keycloak token.
```

(For web/mobile flavors, first build the local TS deps:
`corepack pnpm --dir typescript install --frozen-lockfile && corepack pnpm --dir typescript --filter @baukit/type-guards --filter @baukit/csp --filter @baukit/a11y-core --filter @baukit/analytics-core --filter @baukit/analytics-posthog-native --filter @baukit/api-runtime --filter @baukit/auth-native --filter @baukit/auth-node --filter @baukit/auth-web --filter @baukit/pwa-web --filter @baukit/data-contracts --filter @baukit/ui-tokens --filter @baukit/navigation --filter @baukit/purchases-core --filter @baukit/purchases-revenuecat run build`.)

<!-- baukit:agent-rules:start -->
## Shared agent rules (baukit)

`baukit check agent-docs` keeps this block identical to `lint/agent-rules.md` in baukit.
Edit it there, not here.

### Done means the gate passed

- Run the repo gate before you call a task done. Rust tests run with `-- --include-ignored`, so the
  Docker-gated tests run too. A red gate is never "unrelated" until you have read the failure.
- `baukit check` must pass. Every rule it enforces is listed below.

### Code

- Production code does not panic. `unwrap`, `expect`, `panic!`, indexing and lossy `as` casts are lint
  errors outside tests. Propagate the error or encode the invariant in the type (`let ... else`, `get`,
  `try_from`). `unwrap_or_default` or `.ok()` on a failure that is a bug is not a fix.
- Suppressions carry a reason that names the invariant. Use `#[expect(lint, reason = "...")]`, not
  `#[allow]`. Comment forms (`eslint-disable`, `ast-grep-ignore`, `@ts-expect-error`, `# noqa: CODE`,
  `# nosec`) need `-- reason`; `@ts-ignore` is forbidden. The counts in `baukit-check.toml` only go down.
- No bulk suppression: no `eslint-suppressions.json` or `--suppress-all`, no clippy `-A`/`--cap-lints`
  in gate scripts, no lint set to `allow` in `[lints]` or `.cargo/config.toml` rustflags.
- No file grows past 1000 lines (Rust) or 500 lines (TS). Allowlisted files may not grow at all: split them.
- Domain logic stays free of frameworks. I/O sits behind a port with an adapter, and services orchestrate.
- TypeScript parses untrusted data with guards (`@baukit/type-guards`), never `as unknown as T`.

### Security

- SQL on tenant data puts the owner in the WHERE: `WHERE owner_id = $1 AND id = $2`. Loading by id and
  comparing the owner in Rust is a finding. Every route that takes an id gets a test where a second
  principal is denied.
- Every route sits inside the rate limiter, including unauthenticated callbacks, webhooks and
  `/platform/*` routes merged late.
- Outbound HTTP goes through `baukit-egress`. Do not build a raw `reqwest::Client`.
- Types holding secrets never derive `Debug`. Use `secrecy` or a redacting impl. Never log a token.
- Browser code never writes a token to `localStorage`, `sessionStorage`, IndexedDB or caches.
- New dependencies must pass `cargo deny` and `osv-scanner`. Actions are pinned by commit SHA.
- `docs/threat-model.md` lists every trust boundary. When you add a boundary (route, webhook, integration,
  queue, file upload), update it with the `baukit-threat-model` skill and test the controls with
  `baukit-security-testing`.
<!-- baukit:agent-rules:end -->

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
  must keep the generated fixture green in all CI flavors
  (backend, web, mobile, mobile-pwa, mobile-pwa-auth, combined, mcp, auth, clerk, workos,
  platform-standalone, platform).
