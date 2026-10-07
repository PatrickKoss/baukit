# baukit-mcp

Mount an OAuth-protected MCP resource at `/mcp` in an Axum backend.
`router` validates configuration, discovers the issuer through baukit-auth,
and verifies every token for the configured resource URL. HTTP is allowed
only for loopback development. Host and Origin lists use exact values.

Implement `ToolService` in a product adapter. Each `ScopedTool` declares its
schema and required scopes. The transport checks scopes before execution,
returns HTTP 403 with an OAuth challenge, and passes the effective `Principal`
to the service. `tools/list` lists only permitted tools. The stateless
transport verifies each request and does not retain an authenticated session.

Pass `McpServices::new(tools)` as the second argument to `router`. Register
optional `ResourceService` and `PromptService` adapters with `with_resources`
and `with_prompts`. Their lists filter by scope, and reads and gets use the
same HTTP scope challenges as tools. `service_schema(&services)` exports all
registered definitions for drift checks.

Pass a baukit-ratelimit store and an `AuthenticationPolicy` to `router`.
`JwtOnlyPolicy` preserves verified JWT identity and grants.
`KeycloakIntrospectionPolicy` checks live token state with a timeout and a
bounded token-hash cache. Disable caching for per-request revocation checks.
Product policies can add account lookups, erasure fences and quotas, then
return an effective principal for both discovery and execution.

Use Redis across replicas and the bounded memory store in local development. `tool_schema` exports the
registered schemas and scopes for a product's committed drift artifact.

See [remote MCP](../../../docs/remote-mcp.md) for template setup and client configuration.
