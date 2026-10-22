# MCP capability

`baukit new NAME --backend --mcp --auth oidc` adds a Rust `NAME-mcp` crate to
the backend workspace. Set `mcp = true` under `[capabilities]` in `baukit.toml`.
The backend serves Streamable HTTP at `/mcp` when the runtime configuration
enables it. Clients obtain resource-specific OAuth tokens through Keycloak.

See [remote MCP](../remote-mcp.md) for configuration, tool ports, schema drift
checks and client setup. Existing products must follow the
[migration guide](../migrations/mcp-stdio-to-remote.md).

Products using suite links can enable `baukit-mcp`'s `suite` feature and call
`McpServices::with_suite_links(Arc<SuiteLinkService>)`. The opt-in
`list_suite_links` tool requires `links:read` and returns only the effective
product owner's link metadata in pages of 100. The authentication policy must
map the verified identity to the product account UUID before tool dispatch.
See the [links tool contract](../remote-mcp.md#read-suite-links-with-mcp-tokens)
for the JSON shape, cursor and manifest export.

Generated OIDC REST wiring accepts an optional `auth.authorized_parties` list
of session client IDs. A nonempty list requires a matching `azp`, so tokens
from foreign OAuth clients cannot become REST sessions even if they carry the
REST audience. The default is empty. Keep assistant tokens on the separate MCP
resource audience and grant only the needed tool scopes.
