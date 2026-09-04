# Agent guide

This repository is a Baukit product generated with the `{{ "strict" if context.quality_strict else "standard" }}` quality profile. `baukit.toml` records the enabled capabilities and quality settings.

## Commands

Run commands from the repository root unless a command changes directory.

```sh
sh scripts/setup.sh
sh scripts/preflight.sh
{% if context.backend %}cargo fmt --manifest-path backend/Cargo.toml --all --check
cargo clippy --manifest-path backend/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path backend/Cargo.toml -- --include-ignored
cargo test --manifest-path backend/Cargo.toml -p {{ context.app_name }}-bin --test openapi_drift
{% endif %}{% if context.web %}corepack pnpm@11.18.0 --dir web install --frozen-lockfile
corepack pnpm@11.18.0 --dir web build
corepack pnpm@11.18.0 --dir web lint
corepack pnpm@11.18.0 --dir web test
{% endif %}{% if context.mobile %}corepack pnpm@11.18.0 --dir mobile install --frozen-lockfile
corepack pnpm@11.18.0 --dir mobile typecheck
corepack pnpm@11.18.0 --dir mobile lint
corepack pnpm@11.18.0 --dir mobile test
{% endif %}{% if context.mcp %}corepack pnpm@11.18.0 --dir mcp install --frozen-lockfile
corepack pnpm@11.18.0 --dir mcp build
corepack pnpm@11.18.0 --dir mcp typecheck
corepack pnpm@11.18.0 --dir mcp lint
corepack pnpm@11.18.0 --dir mcp test
corepack pnpm@11.18.0 --dir mcp openapi:check
corepack pnpm@11.18.0 --dir mcp docs:check
{% endif %}baukit doctor
```

`scripts/setup.sh` creates each local `.env` from its matching `.env.example` and appends newly added example assignments on later runs. It never replaces existing bytes. Existing assignments, including blank and exported assignments, count as local choices. If either file repeats a key, the first example assignment is the append candidate and any existing occurrence suppresses the append.

{% if context.quality_strict %}Run the complete strict profile in CI order with:

```sh
sh scripts/quality-gate.sh
```

The strict gate checks local file links in committed Markdown under `README.md`, `CLAUDE.md`, `AGENTS.md`, and `docs/`. It ignores external URLs and fragments. Pass a different list of repository-relative files or directories to `scripts/check-markdown-links.py` when product documentation lives elsewhere.

The strict runner requires the tools used by the enabled capabilities. Backend coverage needs `cargo-llvm-cov`, `cargo-nextest`, and Docker. Browser checks need the Playwright Chromium and WebKit binaries. Native checks need the Android SDK and Java 21. The iOS check prebuilds the native project and bundles JavaScript on Linux. It does not compile Objective-C or Swift.

Set `BAUKIT_BASE_REVISION` to the pull request base commit when running the migration guard outside CI. The default is the merge base with `origin/main`, then `HEAD^`, then the first commit. Add critical Playwright spec paths to `quality.critical_paths` in `baukit.toml`; the strict gate runs them on both WebKit projects `quality.webkit_repeats` times. `quality.full_stack_e2e` is false by default. When a product turns it on, it must provide an executable `scripts/full-stack-e2e.sh` that owns its services, fixtures, and cleanup.

{% endif %}## Dependency and generated-file rules

Commit `Cargo.lock`{% if context.web %}, `web/pnpm-lock.yaml`{% endif %}{% if context.mobile %}, `mobile/pnpm-lock.yaml`{% endif %}{% if context.mcp %}, `mcp/pnpm-lock.yaml`{% endif %}. CI uses `--locked` or `--frozen-lockfile`. Use Corepack's pinned pnpm version. Do not use a globally installed pnpm.

{% if context.backend %}The Rust workspace declares its minimum supported Rust version in `backend/Cargo.toml`. Keep code compatible with that version. Regenerate `backend/openapi.json` with `sh scripts/openapi.sh`. List every committed TypeScript declaration in `openapi.consumers` in `baukit.toml`, then regenerate all of them with `sh scripts/openapi-client.sh`.

Applied files under `backend/migrations/` are immutable. Change the schema with a new forward migration. The strict profile compares existing migration files with the pull request base.

{% endif %}{% if context.web %}The Playwright suite runs Chromium and WebKit at desktop and mobile sizes. Keep the axe, keyboard, overlay, route-state, submit-guard, geometry, console-warning, and scroll specs active. Extend `web/e2e/qa.config.ts` when product routes or states change.

If `capabilities.pwa` becomes true, provide `web`'s `build:sw:check` command and commit its generated service-worker output. The strict runner treats drift as a failure.

{% endif %}{% if context.mobile %}Run Expo Doctor after changing Expo packages or app configuration. Changes to native dependencies, config plugins, or `mobile/app.config.ts` require the iOS bundle check and Android `assembleDebug`.

{% endif %}{% if context.mcp %}The MCP package keeps read and write tools in separate registries. Each tool needs complete annotations and a matching entry in `mcp/src/tool-routes.ts`. Run the OpenAPI and generated-doc checks after changing a registry or route. Keep stdout for protocol messages and send outcome-only logs to stderr.

{% endif %}## Boundaries

{% if context.backend %}The backend uses hexagonal layers. Domain code owns rules and validation. Ports define boundary traits. Services coordinate use cases. API, PostgreSQL, and other adapters implement boundaries. The binary crate owns configuration and composition. Do not import Axum or SQLx into the domain crate, or adapter crates into services.

{% if context.auth_oidc %}Keep provider-specific authentication behavior inside the OIDC adapter. Other backend code depends on verified identity, not provider names or claims.

{% endif %}Run migrations as a release operation. Do not run them during API startup.

{% endif %}{% if context.web or context.mobile %}User-visible limits come from the root `limits.json`. Do not duplicate numeric limits in components.

Use semantic tokens from `@baukit/ui-tokens` for colors, spacing, typography, and motion. Do not add raw color values to components.

{% endif %}If the product enables offline sync later, every synced row needs an owner, stable client-generated ID, revision, `updated_at`, and soft-delete marker. Allocate the owner's next revision in the same transaction as each write. Local reads and writes stay inside the active identity partition, and the UI must not report success while pending or rejected mutations remain.

Full-stack external-service E2E is required only for changes that cross a real adapter boundary or alter the product-owned full-stack harness. Unit tests and hermetic browser tests remain the default for other changes.
