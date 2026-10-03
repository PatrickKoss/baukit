# Changelog

## [Unreleased]

- Fixed doctor checks for custom literal ports, loopback compose mappings, PKCE check paths, and MCP URLs supplied by the environment. Products with a port offset still get checks for stale literal defaults.
- Accepted root pnpm workspaces for web and mobile apps. Allowed `@baukit/auth-node` in web dev dependencies for Keycloak tests and rejected it in web runtime dependencies.
- Fixed snapshot regeneration from the repository root with `--manifest-path cli/Cargo.toml`.
