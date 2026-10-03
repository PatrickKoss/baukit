# Changelog

## [Unreleased]

- Use caret requirements for CLI and generated backend third-party Rust dependencies. Generated Baukit dependencies stay exact.

- Inject auth erasure dependencies in path and registry modes.
- Pin generated development databases to PostgreSQL `18.6-alpine`. Keep the
  volume at `/var/lib/postgresql` for the versioned data directory and document
  local volume recreation on major upgrades.
- Fixed doctor checks for custom literal ports, loopback compose mappings, PKCE check paths, and MCP URLs supplied by the environment. Products with a port offset still get checks for stale literal defaults.
- Accepted root pnpm workspaces for web and mobile apps. Allowed `@baukit/auth-node` in web dev dependencies for Keycloak tests and rejected it in web runtime dependencies.
- Fixed snapshot regeneration from the repository root with `--manifest-path cli/Cargo.toml`.
