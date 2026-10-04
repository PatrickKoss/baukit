# Changelog

## [Unreleased]

- Localize generated navigation labels in English and German.
- Test generated Android QA setup for Chrome first-run preparation and emulator config keys with spaces.
- Add regression coverage for reordered nullable primitive unions and type arrays. The strict template calls the repository checker, which already compares union members by identity.
- Confirm published Rust crates allow compatible third-party updates. The dependency requirement check already rejects exact third-party pins and permits exact Baukit dependencies.

- Refresh all example lockfiles linked to local TypeScript packages during release preparation. Check them with frozen pnpm resolution in CI.

- Follow consumed product identities instead of requiring product.ts or a backend library PRODUCT constant. Accept product display slugs and shared config namespaces. Keep missing bindings and identity drift detectable.

- Cut release sections in the common generated changelog. Put shipped 0.7.0 and 0.6.0 entries in their dated sections and remove the superseded pnpm pin.

- Install the unpublished CLI from the matching Git release tag. Check install commands and update concrete tags during release preparation.

- Check MCP read and write registry exports instead of requiring the template helper module filename. Accept explicit tool-name registries used by existing products. Ignore declarations in comments and strings.

- Stop requiring template guidance filenames. Keep machine-read files such as MCP tool docs and declared OpenAPI consumers required.

- Check explicit host and container port declarations independently. Keep offset defaults for undeclared ports and reject wrong Compose targets and loopback URLs.

- Let products set `capabilities.analytics = "none"` to omit the mobile PostHog adapter. Generated manifests default to `"posthog"`.

- Check that a root pnpm workspace includes each mobile, web, and MCP app when it has no nested workspace.

- Install Corepack 0.36.0 before using pnpm in generated CI. Node 26 does not bundle Corepack.

- Refresh generated dependencies: pnpm 12.9.1, ESLint 10.12, MCP SDK 1.32, Tokio 1.53.2, and UUID 1.27. Match mobile Jest types to Jest 29. Update the MCP doctor check and GitHub Actions patch pins. Use Node 26.10 and matching Node types in generated CI.

- Use caret requirements for CLI and generated backend third-party Rust dependencies. Generated Baukit dependencies stay exact.

- Inject auth erasure dependencies in path and registry modes.
- Pin generated development databases to PostgreSQL `18.6-alpine`. Keep the
  volume at `/var/lib/postgresql` for the versioned data directory and document
  local volume recreation on major upgrades.
- Fixed doctor checks for custom literal ports, loopback compose mappings, PKCE check paths, and MCP URLs supplied by the environment. Products with a port offset still get checks for stale literal defaults.
- Accepted root pnpm workspaces for web and mobile apps. Allowed `@baukit/auth-node` in web dev dependencies for Keycloak tests and rejected it in web runtime dependencies.
- Fixed snapshot regeneration from the repository root with `--manifest-path cli/Cargo.toml`.
