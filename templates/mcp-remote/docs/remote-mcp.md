# Remote MCP

Generate a Rust MCP crate inside the backend with:

```sh
baukit new {{ context.app_name }} --backend --mcp --auth oidc
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
cargo run --manifest-path backend/Cargo.toml -p {{ context.app_name }}-mcp --bin mcp-tools > backend/mcp-tools.json
cargo test --manifest-path backend/Cargo.toml -p {{ context.app_name }}-mcp --test tool_drift
```

## Connect clients

For Claude Code, register the generated public client and fixed callback port:

```sh
claude mcp add --transport http --client-id {{ context.app_name }}-mcp --callback-port 18888 {{ context.app_name }} https://mcp.example.com/mcp
```

Run `/mcp` and authenticate. The public client has no secret.
The registered loopback callback is `http://localhost:18888/callback`.
See [Claude Code MCP configuration](https://code.claude.com/docs/en/mcp).

In Claude Desktop, open Customize > Connectors, add a custom connector with
the public HTTPS `/mcp` URL, select sign-in and "Use your own OAuth client",
and enter `{{ context.app_name }}-mcp` with no secret. The template registers
`https://claude.ai/api/mcp/auth_callback`. Remote connectors connect through
Anthropic's servers, so the resource and issuer must be reachable there.
See [remote connector configuration](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp).

## OAuth flow

A request without a valid token receives 401 with a `resource_metadata` URL.
The client reads RFC 9728 metadata, discovers Keycloak, and signs in with
Authorization Code and PKCE S256. It sends the resource URL in authorization
and token requests and sends the access token on every MCP request.
The backend checks signature, issuer, resource audience, and expiry, then
runs the authentication policy before discovery or execution.
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
there is no authenticated session cookie. The generated `authentication_policy`
function returns `JwtOnlyPolicy`, so revocation takes effect at JWT expiry.
Use the policy port below when a product needs live checks.

## Authentication policy

The product MCP crate's `authentication_policy` function supplies the fourth
argument to `baukit_mcp::router`. Replace that function to select a different
policy. `AuthenticationPolicy::authenticate` receives the verified
`baukit_auth::Principal` and bearer token on every authenticated request.
It returns the effective `baukit_mcp::Principal`, or `PolicyDenial`.
Both `tools/list` and `tools/call` use its effective scopes. Scope reductions
cannot grant rights absent from the JWT or restore rights a prior policy removed.

For Keycloak introspection, add `baukit-config.workspace = true` to the product
MCP crate and load a confidential client's credentials from product config:

```rust
use std::{sync::Arc, time::Duration};
use baukit_config::Secret;
use baukit_mcp::{
    AuthenticationPolicy, KeycloakIntrospectionConfig,
    KeycloakIntrospectionPolicy, McpConfigError,
};

pub fn authentication_policy(
    issuer: &str,
    client_id: &str,
    secret: Secret<String>,
) -> Result<Arc<dyn AuthenticationPolicy>, McpConfigError> {
    let mut config = KeycloakIntrospectionConfig::new(issuer, client_id, secret);
    config.cache_ttl = Duration::ZERO;
    Ok(Arc::new(KeycloakIntrospectionPolicy::new(config)?))
}
```

Pass these arguments and propagate the result in the backend's router setup.
Keep the public PKCE client for MCP clients. Add a separate confidential
Keycloak client with client authentication enabled for the backend's
introspection requests. Store its secret in the deployment secret store.
It needs no interactive flow, password grant, or service-account token flow.
Keycloak restricts its introspection endpoint to confidential clients.
Add a second audience mapper to the MCP public client with
`included.client.audience` set to this backend client ID. Keep the original
mapper with `included.custom.audience` set to the canonical MCP resource URL.
Use separate mappers; Keycloak prefers the client audience if both settings
are present on one mapper. Keycloak 26.8 requires the introspecting client
to be in the token audience. See the
[Keycloak upgrading guide](https://www.keycloak.org/docs/26.8.0/upgrading/#_migrating_to_26_6_2).
See [Keycloak introspection](https://www.keycloak.org/securing-apps/oidc-layers)
and [RFC 7662](https://www.rfc-editor.org/rfc/rfc7662).

The adapter posts the token with HTTP Basic client authentication. It verifies
active status, subject, expiry, and any returned issuer and client ID. It
intersects returned scopes with verified JWT grants; an omitted scope grants
nothing. Errors and timeouts return 503 `authentication_policy_unavailable`.
There is no fallback to JWT grants on provider failure. Inactive or revoked
tokens return 401 `invalid_token` with protected resource metadata.

Defaults are a two-second timeout, a five-second cache lifetime, and at most
1,024 cached successful results. The cache uses SHA-256 token hashes, never raw
tokens. Lifetime cannot exceed 30 seconds or the introspected expiry. Hits do
not extend it. Failed and inactive results are not cached. Set `cache_ttl` to
zero for every-request revocation checks, as SLS requires. A nonzero lifetime
allows revoked sessions to remain usable until the cached result expires.

A product policy can compose the introspection adapter with account service
ports. Check erasure fences using the verified issuer and subject, call
introspection, resolve an active linked account, and consume the product quota.
Return `effective.with_subject(account_id)` to pass the product account ID to
tools. `verified_identity()` retains the original identity for auditing and
ownership checks. Keep fences and account checks outside the introspection
cache so each request observes changes in product state.

Return `PolicyDenial::Inactive` for an erased or deleted account,
`InsufficientScope(required_scopes)` for a missing policy grant,
`Unavailable` for failed account or provider checks, or `RateLimited(duration)`
for product quotas. Missing grants return 403 `insufficient_scope` with the
required scopes in `WWW-Authenticate`; quota denials return 429 with
`Retry-After`. Policy implementations use domain types and service ports,
with no Axum or rmcp types.

The generated `services` function returns tools-only `McpServices` for the
router's second argument. Add optional resource and prompt services there.
`ItemTools::schema` and `tool_drift` also record empty resource and prompt
registries. When adding either service,
include its definitions in that export and review `mcp-tools.json`.
Use `ToolError::new` for default errors or `with_structured_content` for a
product error envelope. Test structured errors against the product's output
schema. Use `text_only()` to preserve text-only `isError` results with the
existing output schema. See [Baukit remote MCP](https://github.com/PatrickKoss/baukit/blob/main/docs/remote-mcp.md)
for optional service ports and examples.
