# ADR 0019: MCP servers are remote only, as OAuth protected resources

## Status
Accepted, 2026-11-27.

## Context
A local stdio MCP server runs with the user's own credentials on their machine, outside the backend's authorization, rate limits and audit. Each product would also ship a second TypeScript server next to its Rust backend.

The tools need the same scopes and owner checks as the product's HTTP API, which already lives in the backend.

## Decision
`baukit-mcp` mounts MCP at `/mcp` in the product's Axum backend. `baukit_mcp::router` takes an `IdentityVerifier`, `McpServices`, a rate-limit store and an `AuthenticationPolicy`. It verifies every request, serves RFC 9728 metadata at `/.well-known/oauth-protected-resource`, checks each `ScopedTool` scope before dispatch and keeps no session. `JwtOnlyPolicy` and `KeycloakIntrospectionPolicy` are the shipped policies. See the [crate README](../../rust/crates/baukit-mcp/README.md) and the [MCP guide](../guide/mcp.md).

`baukit new --mcp` requires `--backend` and `--auth oidc`, `clerk` or `workos`. There is no stdio template.

## Enforced by
- `mcp_requires_a_backend_and_auth_provider` and `remote_mcp_requires_auth_and_backend` in `cli/tests/generator.rs`. Gate step `cli-tests`.
- `metadata_and_oauth_errors_follow_the_resource_contract` and `body_limit_rate_limit_and_authority_rejection_are_enforced` in `rust/crates/baukit-mcp/tests/http.rs`. Gate step `rust-tests`.
- The `mcp-fixture` gate step calls a generated server over HTTP with a real Keycloak token.

## Consequences
Products get one MCP surface behind their own auth, limits and scopes. Clients need an OAuth flow or a personal access token. Offline local tools are product work.

## History
- 2026-10-06, `6eaa6374`: OAuth-protected remote Rust servers added.
- 2026-10-07, `c09429c4`: the TypeScript stdio template and its flags removed. Remote Rust became the only MCP template.
