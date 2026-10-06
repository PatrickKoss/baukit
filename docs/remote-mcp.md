# Remote MCP

Generate a Rust MCP crate inside the backend with:

```sh
baukit new my-product --backend --mcp --mcp-transport remote --auth oidc
```

The backend mounts `/mcp` when `mcp.enabled` is true. Configure
`MCP__RESOURCE_URL`, `MCP__ISSUER`, `MCP__ALLOWED_HOSTS`, and
`MCP__ALLOWED_ORIGINS` under the product's environment prefix.
Lists are JSON arrays. Use the canonical public `/mcp` URL as the resource.
The Helm `mcp` values configure the resource, issuer, and allowlists. The chart
adds `/mcp` and protected resource metadata paths when ingress is enabled.
Update the Keycloak MCP audience mapper to the same public resource URL.
Use HTTPS outside loopback development and a shared Redis rate-limit store
outside local development. The generated template starts with MCP disabled.

## Add a tool

Add a service trait and implementation in the product's ports and services,
or beside the MCP adapter when only MCP consumes it. Tool logic takes domain
types and calls that service trait. The MCP adapter converts JSON arguments,
maps `Principal` to the product identity, and converts results to JSON.
The generated `ItemReadService` demonstrates this boundary. Its sample reads
the shared item catalog. Products with user-owned records must scope the
service query to the verified issuer and subject.

Add a `ScopedTool` to `ItemTools::definitions` with its input and output schemas,
read-only annotation, and required OAuth scopes. Add its dispatch in `call`.
The HTTP layer requires all declared scopes before execution. `tools/list`
omits tools the principal cannot call. Product authorization still belongs
in the service. Add the scope to Keycloak and test the service and HTTP call.

Export the contract and review its diff:

```sh
cargo run --manifest-path backend/Cargo.toml -p my-product-mcp --bin mcp-tools > backend/mcp-tools.json
cargo test --manifest-path backend/Cargo.toml -p my-product-mcp --test tool_drift
```

## Connect clients

For Claude Code, register the generated public client and fixed callback port:

```sh
claude mcp add --transport http --client-id my-product-mcp --callback-port 18888 my-product https://mcp.example.com/mcp
```

Run `/mcp` and authenticate. The public client has no secret.
The registered loopback callback is `http://localhost:18888/callback`.
See [Claude Code MCP configuration](https://code.claude.com/docs/en/mcp).

In Claude Desktop, open Customize > Connectors, add a custom connector with
the public HTTPS `/mcp` URL, select sign-in and "Use your own OAuth client",
and enter `my-product-mcp` with no secret. The template registers
`https://claude.ai/api/mcp/auth_callback`. Remote connectors connect through
Anthropic's servers, so the resource and issuer must be reachable there.
See [remote connector configuration](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp).

## OAuth flow

A request without a valid token receives 401 with a `resource_metadata` URL.
The client reads RFC 9728 metadata, discovers Keycloak, and signs in with
Authorization Code and PKCE S256. It sends the resource URL in authorization
and token requests and sends the access token on every MCP request.
The backend checks signature, issuer, resource audience, and expiry.
A missing tool scope receives 403 `insufficient_scope` with the required scopes.
These requirements follow [MCP authorization 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

The template uses pre-registration. It does not enable anonymous dynamic
registration or depend on Client ID Metadata Documents. Pre-registration works
with Keycloak's realm import and keeps redirect URIs and granted scopes under
product control. Keycloak's custom audience mapper binds this client to one
resource even where Keycloak ignores the OAuth `resource` parameter.

rmcp 3.5.1 serves stateless HTTP and supports the 2026-07-28 protocol, which uses
`server/discover` and per-request metadata. It also accepts the 2025-11-25 and
2025-06-18 revisions with `initialize`. Tokens are verified for every request;
there is no authenticated session cookie. Revocation takes effect when the
JWT expires. Products that need immediate revocation should check their
identity or grant state in the service adapter.
