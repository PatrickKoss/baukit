---
name: baukit-new-product
description: Scaffold a new Baukit product with the canonical CLI and complete its first local verification. Use when creating a product repository, selecting backend, Expo mobile, or Vite web capabilities, or regenerating missing template-owned files intentionally.
---

# Scaffold a Baukit product

Treat the `baukit` CLI as the source of truth. Never hand-scaffold files or directories that `baukit new` generates.

## Preflight

1. Choose a lowercase kebab-case product name and at least one capability:
   - `--backend`: Rust workspace with domain, ports, services, API, PostgreSQL adapter, binaries, migrations, tests, local Compose, deploy values, and CI.
   - `--mobile`: Expo/React Native application under `mobile/`.
   - `--web`: Vite/React/TanStack Query application under `web/`.
2. Check the required tools:

   ```sh
   command -v cargo
   command -v pnpm
   command -v baukit
   cargo --version
   pnpm --version
   baukit --version
   ```

   Cargo is required for a backend and for installing the CLI. Node.js plus pnpm is required for mobile/web work and TypeScript OpenAPI generation.

3. If `baukit` is absent, install it from a trusted checkout or a pinned release train:

   ```sh
   cargo install --path /path/to/baukit/cli
   cargo install --git https://github.com/patrickkoss/baukit.git --tag vX.Y.Z --bin baukit baukit-cli
   ```

## Generate

Run from the directory that should contain the product:

```sh
baukit new <product-name> --backend --mobile --web --dir <parent-directory>
```

Keep only the selected capability flags. `--dir` is the parent; the CLI creates `<parent-directory>/<product-name>`. For local Baukit development, add `--baukit-path /path/to/baukit/rust`. Use `--force` only when intentionally adding missing template files to a non-empty destination; it does not overwrite differing files and reports conflicts.

When another generated product already uses the default local ports, add
`--port-offset N`. Use a different non-negative offset for each product. The
CLI applies it to the generated service ports and records it in `baukit.toml`
for `baukit doctor`.

The product root contains `baukit.toml`. Selected capabilities add `backend/`, `mobile/`, and/or `web/`; backend generation also adds the shared root Makefile, CI, Compose, deployment values, and OpenAPI workflow.

The mobile target uses Expo Router. `mobile/app/_layout.tsx` owns startup providers and the root stack, `mobile/app/(tabs)/_layout.tsx` owns primary navigation, and `mobile/app/(tabs)/index.tsx` is the initial screen. `mobile/src/app-preferences.ts` persists language, theme, and analytics consent through the generated record store. OIDC generation keys those preferences by subject and resets visible preferences on sign-out. It also adds `mobile/app/(auth)/sign-in.tsx` and gates the route groups in the root layout. Add screens as files under the appropriate route group. Use `mobile/src/back-or-replace.ts` for a deep-linkable screen's back action, with its semantic parent as the fallback.

## Verify the first checkout

1. Enter the generated product and initialize version control:

   ```sh
   cd <parent-directory>/<product-name>
   git init
   ```

2. Build each selected capability:

   ```sh
   cargo check --manifest-path backend/Cargo.toml --workspace --all-targets
   pnpm --dir mobile install
   pnpm --dir mobile typecheck
   pnpm --dir web install
   pnpm --dir web build
   ```

   Run only commands for generated directories.

3. From the product root, validate the manifest, dependency source, and expected generated files:

   ```sh
   baukit doctor
   ```

4. Fix reported toolchain or dependency problems; do not replace failed generation with handwritten scaffolding. Follow the matching release contracts in the Baukit checkout, especially `docs/platform/baukit-conventions.md`.
