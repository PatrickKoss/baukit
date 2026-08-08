---
name: baukit-upgrade
description: Move a generated product to a newer coordinated Baukit release train and verify dependency, template, API, backend, mobile, and web compatibility. Use when updating pinned Baukit git tags, responding to Renovate release-train changes, or resolving `baukit doctor` version drift.
---

# Upgrade a Baukit product

Upgrade every Baukit component as one release train. Do not invent or call a `baukit upgrade` command; the implemented CLI validates the result with `doctor` and regenerates OpenAPI TypeScript declarations.

## Prepare

1. Start with a clean worktree and record the current `baukit.toml` capabilities and dependency source.
2. Read the target release notes and the target checkout's:
   - `docs/platform/compatibility-matrix.md`
   - `docs/platform/baukit-conventions.md`
3. Install the CLI from the same target tag when necessary:

   ```sh
   cargo install --git https://github.com/patrickkoss/baukit.git --tag baukit-vX.Y.Z --bin baukit baukit-cli
   ```

## Update the train coherently

1. Change every Baukit git dependency in `backend/Cargo.toml` from the old tag to the target tag. Keep all `baukit-*` crates on exactly the same tag.
2. Change every `@baukit/*` git dependency in `mobile/package.json` and/or `web/package.json` to that same tag while preserving each package's `path:typescript/packages/...` selector.
3. Update `baukit.toml` so `template_version` and `[dependencies.baukit].tag` describe the target train. Apply release-note migrations to template-owned files deliberately; preserve product-owned behavior and surface conflicts for review.
4. Refresh Cargo and pnpm lockfiles. Do not mix unrelated dependency upgrades into this change, and do not switch to public registries while the project remains private-first.

## Validate and verify

Run from the product root:

```sh
baukit doctor
```

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
