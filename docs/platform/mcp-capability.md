# MCP capability

`baukit new NAME --backend --mcp --auth oidc` adds a Rust `NAME-mcp` crate to
the backend workspace. Set `mcp = true` under `[capabilities]` in `baukit.toml`.
The backend serves Streamable HTTP at `/mcp` when the runtime configuration
enables it. Clients obtain resource-specific OAuth tokens through Keycloak.

See [remote MCP](../remote-mcp.md) for configuration, tool ports, schema drift
checks and client setup. Existing products must follow the
[migration guide](../migrations/mcp-stdio-to-remote.md).
