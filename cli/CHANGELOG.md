# Changelog

## [Unreleased]

## [0.9.0] - 2026-10-07

- Check MCP wiring across declared Cargo modules and accept explicit Doctor source paths.

- Register generated MCP services through `McpServices`.

- Cut CLI release notes during release preparation.
- Check tool, resource and prompt definitions in generated MCP drift tests. Let the tools-only example return schema-conforming errors.

- Generate OIDC, Clerk and WorkOS authentication for backend, web, mobile and MCP products. Doctor checks provider configuration and SDK wiring.

## [0.8.0] - 2026-10-07

- Include the command arguments in Android QA timeout errors.
- Supply localized web menu Close labels and the native navigation danger token in generated shells.
- Wire an explicit JWT-only authentication policy in generated remote MCP crates. Document introspection and product account policies.

- Breaking: `--mcp` now generates the Rust remote MCP server and requires `--backend --auth oidc`. Remove the TypeScript stdio template and transport and authentication selection flags. Existing MCP manifests and TypeScript servers receive a migration finding with `docs/migrations/mcp-stdio-to-remote.md`.

## Earlier releases

These entries were already present in v0.7.4.

- Add `--pwa` to generate a worker builder and dependency for the selected web host, including mobile-only products.
- Apply mobile top insets, select iOS Release builds, split OIDC storage by platform, and remove token-expiry waits from generated tests.

- Reject release preparation when the template changelog has uncut Unreleased entries or lacks the new release heading. Restore the missing 0.7.2 template heading from release history.

- Follow relative imports from mobile routes when checking sign-in wiring. Recognize multiline JSX tags. Bound traversal and stop import cycles.
- Scan tracked files and untracked files that Git does not ignore. Keep the filesystem scan outside Git, including when Git is not installed.

- Accept Redis URL environment fallbacks and detect MCP router and tool wiring by content. Use declared OpenAPI consumers instead of a fixed MCP schema path.

- Parse balanced parentheses and angle brackets in strict Markdown link targets.
- Stop Android QA setup before cleanup or service startup when the device probe fails or times out.

- Accept product limit validators that use shared measurements with their own bounds and reason codes.

- Follow mobile route re-exports when checking sign-in wiring. Accept reconciliation tests that exercise input validation or realm reconciliation.

- Resolve consumed `crate::PRODUCT` constants in library modules, including imported aliases. Keep binary crate identities separate.
- Accept Keycloak realm names that differ from the application name. Require the optional Python PKCE helper only when its path is declared.
- Confirm registry dependencies do not require a local Baukit path. Keep ambiguous realms and missing scoped mobile persistence as findings.

- Find backend, worker, OIDC and mobile auth wiring in declared Cargo packages and source files. Add doctor path overrides for custom layouts and discover moved Keycloak inputs.
- Respect analytics = "none" without requiring analytics files or analytics-core.
- Require a Redis URL only when backend source uses a Redis-backed feature.
- Scan consumed identities in declared crates, including short names such as sl-bin.
- Stop requiring the optional web Keycloak test helper dependency in doctor. Keep its browser runtime restriction.
- Confirm doctor accepts declared Redis ports and `/v1/me` identity checks. Confirm generated Jest types match Jest 29.
- Test pinned Android tools, AVD image changes, bounded ADB probes, and coverage output with an external Cargo target.
- Keep generated Expo dependency checks enabled for every package.

- Return mobile OIDC callbacks to the app root. Keep other native links unchanged.

- Use Chrome's Android first-run switch during OIDC QA setup.

- Localize generated navigation labels in English and German.
- Test generated Android QA setup for Chrome first-run preparation and emulator config keys with spaces.
- Add regression coverage for reordered nullable primitive unions and type arrays. The strict template calls the repository checker, which already compares union members by identity.
- Confirm published Rust crates allow compatible third-party updates. The dependency requirement check already rejects exact third-party pins and permits exact Baukit dependencies.

- Refresh all example lockfiles linked to local TypeScript packages during release preparation. Check them with frozen pnpm resolution in CI.

- Follow consumed product identities instead of requiring product.ts or a backend library PRODUCT constant. Accept product display slugs and shared config namespaces. Keep missing bindings and identity drift detectable.

- Cut release sections in the common generated changelog. Put shipped 0.7.0 and 0.6.0 entries in their dated sections and remove the superseded pnpm pin.

- Install the unpublished CLI from the matching Git release tag. Check install commands and update concrete tags during release preparation.

- Stop requiring template guidance filenames. Keep declared OpenAPI consumers required.

- Check explicit host and container port declarations independently. Keep offset defaults for undeclared ports and reject wrong Compose targets and loopback URLs.

- Let products set `capabilities.analytics = "none"` to omit the mobile PostHog adapter. Generated manifests default to `"posthog"`.

- Check that a root pnpm workspace includes each mobile and web app when it has no nested workspace.

- Install Corepack 0.36.0 before using pnpm in generated CI. Node 26 does not bundle Corepack.

- Refresh generated dependencies: pnpm 12.9.1, ESLint 10.12, Tokio 1.53.2, and UUID 1.27. Match mobile Jest types to Jest 29. Update GitHub Actions patch pins. Use Node 26.10 and matching Node types in generated CI.

- Use caret requirements for CLI and generated backend third-party Rust dependencies. Generated Baukit dependencies stay exact.

- Inject auth erasure dependencies in path and registry modes.
- Pin generated development databases to PostgreSQL `18.6-alpine`. Keep the
  volume at `/var/lib/postgresql` for the versioned data directory and document
  local volume recreation on major upgrades.
- Fixed doctor checks for custom literal ports, loopback compose mappings and PKCE check paths. Products with a port offset still get checks for stale literal defaults.
- Accepted root pnpm workspaces for web and mobile apps. Allowed `@baukit/auth-node` in web dev dependencies for Keycloak tests and rejected it in web runtime dependencies.
- Fixed snapshot regeneration from the repository root with `--manifest-path cli/Cargo.toml`.
