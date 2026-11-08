---
name: baukit-upgrade
description: Move a generated product to a newer coordinated Baukit release train and verify dependency, template, API, backend, mobile, and web compatibility. Use when bumping Baukit registry pins, responding to Renovate release-train changes, or resolving `baukit doctor` version drift.
---

# Upgrade a Baukit product

Upgrade every Baukit component as one release train. Do not invent or call a `baukit upgrade` command; the implemented CLI validates the result with `doctor` and regenerates OpenAPI TypeScript declarations.

## Prepare

1. Start with a clean worktree and record the current `baukit.toml` capabilities, quality profile, and dependency source.
2. Read the target release notes and the target checkout's:
   - `docs/platform/compatibility-matrix.md`
   - `docs/platform/baukit-conventions.md`
3. The CLI is not on crates.io. Install it from the target tag when necessary:

   ```sh
   cargo install --git https://github.com/PatrickKoss/baukit --tag vX.Y.Z --locked baukit-cli
   ```

## Update the train coherently

1. Set every `baukit-*` crate in `backend/Cargo.toml` (and `backend/fuzz/Cargo.toml` if present) to the target version, for example `baukit-core = "0.19.1"`. All crates move together.
2. Set every `@baukit/*` dependency in `mobile/package.json` and/or `web/package.json` to the exact target version, for example `"@baukit/api-runtime": "0.19.1"`.
3. Update `baukit.toml` so `template_version` and `[dependencies.baukit].version` name the target train, with `source = "registry"`. A product still on `source = "git"` with `tag = "vX.Y.Z"` pins switches to registry pins in the same change; `doctor` reports any pin that differs from `[dependencies.baukit].version`.
   Read every `templates/common/CHANGELOG.md` entry after the current version and the target's section in `docs/platform/compatibility-matrix.md` first. Apply release-note migrations to template-owned files deliberately; preserve product-owned behavior and surface conflicts for review.
   Preserve `[quality]` thresholds, critical paths, the full-stack E2E opt-in, and every `openapi.consumers` entry. Add generated strict-profile scripts only when the product selects `profile = "strict"`.
4. Refresh the managed agent-rules block at the end of `CLAUDE.md` from the target checkout. There is no `baukit upgrade` command for it; `check agent-docs --fix` rewrites the block and turns a copied `AGENTS.md` into the symlink to `CLAUDE.md`:

   ```sh
   cargo run --manifest-path <target-baukit>/cli/Cargo.toml --bin baukit -- check agent-docs --fix --root .
   ```

5. Refresh Cargo and pnpm lockfiles. Do not mix unrelated dependency upgrades into this change.

## Validate and verify

Run from the product root:

```sh
baukit doctor
baukit check agent-docs file-size suppressions --root .
```

`doctor` checks the OpenAPI schema and consumer paths declared in `baukit.toml`.
For workers, any product migration filename is valid if its SQL creates the
`job_outbox` table. Frontend API and auth sources may read their URLs entirely
from environment variables; `doctor` reports those files only when they contain
a `localhost:` URL with the wrong configured port.

Resolve every failure before continuing. Then run the checks for each enabled capability:

```sh
make check
baukit generate openapi-client
(cd backend && cargo test --test openapi_drift)

pnpm --dir mobile install --frozen-lockfile
pnpm --dir mobile typecheck
pnpm --dir mobile lint
pnpm --dir mobile test

pnpm --dir web install --frozen-lockfile
pnpm --dir web build
pnpm --dir web lint
pnpm --dir web test
```

Run only commands for selected capabilities. If a lockfile does not exist yet, run the corresponding install once without `--frozen-lockfile`, review it, then rerun the full product CI equivalent. If Docker is available, also run the ignored PostgreSQL adapter integration test. Review the final diff for one target train, regenerated API artifacts, and only documented migration changes.

For a strict product, finish with `sh scripts/quality-gate.sh`. Set `BAUKIT_BASE_REVISION` to the pre-upgrade commit so the migration guard checks that the upgrade added migrations instead of rewriting applied files.
