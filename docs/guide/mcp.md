# Serve a remote MCP endpoint

At the end of this page your backend serves the Model Context Protocol over Streamable HTTP at
`/mcp`, assistants sign in to it with OAuth, and each tool runs only with the scopes it declares.
It is for product developers who generated a backend with `--mcp` or add `baukit-mcp` to one. baukit
supports remote MCP only, decided in [ADR 0019](../adr/0019-remote-mcp-only.md).

## Generate the MCP crate

```sh
baukit new my-product --backend --mcp --auth oidc   # or --auth clerk, --auth workos
```

The CLI adds a `my-product-mcp` crate to the backend workspace and sets `mcp = true` under
`[capabilities]` in `baukit.toml`. The crate holds the tool adapters, the `mcp-tools` binary that
exports the tool contract to `backend/mcp-tools.json`, and the `tool_drift` test that fails when the
code and the committed contract disagree. The generated `docs/remote-mcp.md` in the product walks
through adding a tool, returning tool errors and adding resources and prompts. Its source is
[`templates/mcp-remote/docs/remote-mcp.md`](../../templates/mcp-remote/docs/remote-mcp.md).

## How the pieces fit

`baukit_mcp::router(config, verifier, services, store, policy)` builds the `/mcp` routes and the
protected resource metadata routes. Its arguments are:

- `config` is a `McpConfig`, loaded from `[mcp]`.
- `verifier` is an `Arc<dyn IdentityVerifier>` that checks the bearer token and its binding to this
  resource.
- `services` is a `McpServices` built from an `Arc<dyn ToolService>`. `with_resources` and
  `with_prompts` add the optional services.
- `store` is the rate-limit store for the per-principal quota.
- `policy` is an `AuthenticationPolicy` that turns the verified principal into the effective one,
  or denies it.

Tool logic sits behind a product service trait and takes domain types. The MCP adapter converts
JSON arguments, maps the `Principal` to the product identity and converts results back. Each
`ScopedTool` declares its input and output schemas, annotations and required OAuth scopes. The
server checks the scopes before dispatch, and `tools/list` omits tools the caller cannot call.
Product authorization, such as ownership of a record, still belongs in the service.

The server runs on rmcp 3.9.0 in stateless HTTP mode. It speaks protocol revision 2026-07-28 and
also accepts 2025-11-25 and 2025-06-18. There is no authenticated session cookie, so every request
carries a token and the server verifies it every time.

## Configure the endpoint

`McpConfig` reads these keys from `[mcp]` or from `<PREFIX>__MCP__*` environment variables. Lists
are JSON arrays in the environment.

| Key | Default | Meaning |
| --- | --- | --- |
| `enabled` | `false` | Mount `/mcp` |
| `resource_url` | empty | The canonical public URL ending in `/mcp`, used as the token audience |
| `issuer` | empty | The authorization server named in the metadata |
| `jwks_uri` | unset | Fetch signing keys here instead of through discovery |
| `oauth_client_id` | unset | Clerk only: pin tokens to one OAuth client |
| `required_scopes` | empty | Clerk only: scopes every token must carry when no client is pinned |
| `introspection_client_id`, `introspection_client_secret` | unset | Keycloak introspection credentials |
| `allowed_hosts` | empty | Exact `Host` values to accept |
| `allowed_origins` | empty | Browser origins to accept |
| `max_request_body_bytes` | 32768 | Request body limit |
| `requests_per_minute` | 60 | Quota per issuer and subject |

`McpConfig::validate` rejects the configuration at startup when the resource URL, issuer or an
origin is not HTTPS or loopback HTTP, when the resource path is not `/mcp`, when `allowed_hosts` is
empty or when the body limit is zero. Use HTTPS outside loopback development and the shared Redis
rate-limit store outside local development.

`jwks_uri` helps when containers cannot reach the public issuer. It changes where keys come from,
not the expected issuer or the authorization server in the metadata. In the generated Compose
`backend` profile the OIDC flavor fetches keys from
`http://keycloak:8080/realms/NAME/protocol/openid-connect/certs` while the issuer stays the
browser-visible URL.

### Chart values

The `baukit-app` chart has an `mcp` block, with these defaults in
`deploy/chart/baukit-app/values.yaml`:

```yaml
mcp:
  enabled: false
  resourceUrl: ""
  issuer: ""
  jwksUri: ""
  allowedHosts: []
  allowedOrigins: []
  maxRequestBodyBytes: 32768
  requestsPerMinute: 60
```

With `enabled: true` the chart sets `MCP__RESOURCE_URL`, `MCP__ISSUER`, `MCP__ALLOWED_HOSTS`,
`MCP__ALLOWED_ORIGINS`, `MCP__MAX_REQUEST_BODY_BYTES`, `MCP__REQUESTS_PER_MINUTE` and, when set,
`MCP__JWKS_URI`. Rendering fails when `resourceUrl`, `issuer` or `allowedHosts` is empty. When ingress
is enabled, the chart adds exact paths for `/mcp`, `/.well-known/oauth-protected-resource` and
`/.well-known/oauth-protected-resource/mcp`. A generated product's `deploy/values.yaml` turns MCP on
with example hosts, while the runtime default stays off.

For the bundled OIDC realm, set the Keycloak MCP audience mapper to the same public resource URL.

## The OAuth flow

1. A request without a valid token gets 401 with a `resource_metadata` URL in `WWW-Authenticate`.
2. The client reads the RFC 9728 protected resource metadata and finds the authorization server.
3. The client signs in with authorization code and PKCE S256, sends the resource URL in the
   authorization and token requests, and then sends the access token on every MCP request.
4. The backend checks signature, issuer, token binding and expiry, then runs the authentication
   policy before discovery or execution.
5. A tool whose scope the token lacks gets 403 `insufficient_scope` with the required scopes.

This follows [MCP authorization 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/authorization).

Identity failures that are not the client's fault do not ask the client to sign in again. Neither
of these responses sends `WWW-Authenticate`:

- A temporary failure answers 503 `{"error":"identity_unavailable"}` with `Retry-After: 5`. This
  covers token store outages, connection and body-read failures, discovery and JWKS timeouts, and
  provider 408, 429 and 5xx responses.
- Invalid provider metadata or JWKS, other provider 4xx responses and server configuration errors
  answer 500 `{"error":"identity_misconfigured"}`.

Every verified request counts against a quota keyed by resource, issuer and subject. Over the quota
the server answers 429 `rate_limited` with `Retry-After`. If the store is down it answers 503
`rate_limit_unavailable`. Rejected bearer tokens count against one shared bucket per resource, so a
flood of bad tokens turns into 429 as well.

The bundled OIDC realm pre-registers its clients. It enables no anonymous dynamic registration and
does not depend on Client ID Metadata Documents. Keycloak's custom audience mapper binds the MCP
client to one resource even where Keycloak ignores the OAuth `resource` parameter.

## Keep REST and MCP tokens apart

The generated backend builds two verifiers: one for REST routes from `[auth]`, and `mcp_verifier`
in `backend/crates/<app>-bin/src/identity.rs` for `/mcp`. Keep assistant tokens on the MCP resource
audience and grant them only the tool scopes they need. A token that also carries the REST audience
can call ordinary product routes unless the REST verifier restricts clients.

`auth.authorized_parties` is that restriction for OIDC. A nonempty list requires a matching `azp`,
so a token from a foreign OAuth client cannot become a REST session even when it carries the REST
audience. The list is empty by default. MCP does not inherit it. The [auth guide](auth.md#restrict-the-clients-with-authorized_parties)
has the details.

## Connect clients

For Claude Code, register a public client with a fixed callback port. The bundled OIDC realm
supplies `my-product-mcp`, with the redirect `http://localhost:18888/callback`:

```sh
claude mcp add --transport http --client-id my-product-mcp --callback-port 18888 my-product https://mcp.example.com/mcp
```

Run `/mcp` in Claude Code and authenticate. The public client has no secret. See
[Claude Code MCP configuration](https://code.claude.com/docs/en/mcp).

In Claude Desktop, open Customize, then Connectors, and add a custom connector with the public HTTPS
`/mcp` URL. Choose "Use your own OAuth client" and enter the client ID with no secret. The bundled
realm registers `https://claude.ai/api/mcp/auth_callback`. Remote connectors connect through
Anthropic's servers, so the resource and the issuer must be reachable from the internet. See
[remote connector configuration](https://support.claude.com/en/articles/11175166-get-started-with-custom-connectors-using-remote-mcp).

### Clerk

Set `mcp.issuer` to the Clerk Frontend API URL, choose JWT access tokens in the OAuth application
settings and require PKCE S256. Clerk does not document how to bind a token to a resource, so the
generated `mcp_verifier` binds it another way:

- With `mcp.oauth_client_id` set, `ClerkOAuthVerifier` checks signature, exact issuer, expiry, the
  signed `client_id` and each tool's scope. Pre-register a public client for this resource only,
  register `items:read` as a custom scope and grant it only to that client.
- Clients that register through Clerk's dynamic registration get a new `client_id` each time. Leave
  `mcp.oauth_client_id` unset and set `mcp.required_scopes`. `ClerkMcpVerifier` then requires a
  nonempty signed `client_id` and every required scope. Give the scopes a product prefix, such as
  `my-product:items:read`, so a token for another product on the same Clerk instance cannot carry
  them.

Startup fails when neither key is set. The generated `config/local.toml` sets
`required_scopes = ["items:read"]` against the local Keycloak realm. See
[Clerk MCP setup](https://clerk.com/docs/expressjs/guides/ai/mcp/build-mcp-server).

### WorkOS

Enable WorkOS Connect, configure the public `/mcp` URL as a resource indicator and register the tool
scopes. Set `mcp.issuer` to the AuthKit domain's OAuth issuer, such as
`https://example.authkit.app`. The generated verifier fetches keys from `<issuer>/oauth2/jwks` and
requires the exact resource URL in `aud`, so a REST token or a token for another resource fails.
Register a Connect client with the MCP client's callback and PKCE S256. See
[WorkOS Connect MCP setup](https://workos.com/docs/authkit/mcp).

## Personal access tokens

To accept PATs at `/mcp` too, wrap the MCP verifier in `baukit_auth::ApiTokenVerifier` before passing
it to the router:

```rust
let verifier = Arc::new(baukit_auth::ApiTokenVerifier::new(
    tokens,
    mcp_verifier(&config.product.auth, &config.product.mcp).await?,
));
let mcp = baukit_mcp::router(config.product.mcp.clone(), verifier, services, store, policy).await?;
```

This snippet is not compiled by any test. `tokens` is the product's `ApiTokenService`. MCP uses a
PAT's grants as its effective scopes, so issue grants that match the registered tool, resource and
prompt scopes. A PAT without grants sees no scoped tools. `baukit_auth::Principal::scopes()` stays
empty for API-token identities.

`ApiTokenVerifier` uses the token owner's UUID as the subject and has no issuer or client ID. If the
product's account checks need another subject, write an `IdentityVerifier` that calls
`ApiTokenService::verify`, resolves the owner and calls
`baukit_auth::Principal::from_api_token(resolved_subject, token)`. That constructor does not check
the token, so the custom verifier must read the current store on each request and reject revoked or
expired tokens. With `ApiTokenVerifier`, the next request sees a revocation.

## Choose an authentication policy

`AuthenticationPolicy::authenticate` receives the verified `baukit_auth::Principal` and the bearer
token on every request. It returns the effective `baukit_mcp::Principal` or a `PolicyDenial`. Both
`tools/list` and `tools/call` use the effective scopes. A policy can narrow scopes with
`with_scopes` but never add or restore them.

- `JwtOnlyPolicy` is the generated default, returned by `authentication_policy` in the MCP crate.
  OIDC revocation then takes effect when the JWT expires.
- `KeycloakIntrospectionPolicy` asks Keycloak on each request, behind a cache. The generated
  `mcp_policy` selects it when both `mcp.introspection_client_id` and
  `mcp.introspection_client_secret` are set, and fails at startup on partial credentials or a
  non-OIDC provider.

The introspection adapter posts the token with HTTP Basic client authentication and intersects the
returned scopes with the JWT's grants. Its defaults are a 2 second timeout, a 5 second cache
lifetime and 1,024 cached results. The cache keys on SHA-256 token hashes, and its lifetime never
exceeds 30 seconds or the token's expiry. Set `cache_ttl` to zero for a check on every request.
Provider errors answer 503 `authentication_policy_unavailable` with no fallback to the JWT grants.
Inactive tokens answer 401 `invalid_token`.

Keycloak only lets confidential clients introspect. Add a confidential backend client with client
authentication and no interactive flow, and store its secret in the deployment secret store. Give
the MCP public client a second audience mapper with `included.client.audience` set to that backend
client ID, and keep the first mapper with `included.custom.audience` set to the resource URL.
Keycloak 26.8 requires the introspecting client in the token audience. See the
[Keycloak upgrading guide](https://www.keycloak.org/docs/26.8.0/upgrading/#_migrating_to_26_6_2).

`KeycloakIntrospectionPolicy` rejects PAT principals as `PolicyDenial::Inactive` without sending them
to Keycloak. A product that accepts both kinds branches first:

```rust
let effective = if principal.api_token().is_some() {
    baukit_mcp::Principal::from(principal.clone())
} else {
    introspection.authenticate(principal, bearer_token).await?
};
```

This snippet is not compiled by any test. A product policy can also check erasure fences, resolve
the linked account, consume a product quota and return `effective.with_subject(account_id)`. Keep
those checks outside the introspection cache. Return `PolicyDenial::Inactive` for an erased account,
`InsufficientScope(scopes)` for a missing grant, `Unavailable` for a failed check and
`RateLimited(duration)` for a product quota. They answer 401, 403, 503 and 429.

## Read suite links from MCP

Products that use suite links can enable the `suite` feature of `baukit-mcp` and register the
opt-in `list_suite_links` tool. No tool is registered by default. From the
[baukit-mcp README](../../rust/crates/baukit-mcp/README.md):

```rust
let services = baukit_mcp::McpServices::new(product_tools)
    .with_suite_links(suite_link_service, suite_owner_resolver);
```

Pass an `Arc<SuiteLinkService>`, or an `Arc<dyn SuiteLinksRead>` adapter, and an
`Arc<dyn SuiteOwnerResolver>`. The resolver maps `principal.verified_identity()` to an admitted
product account and returns `None` for an identity without one. The tool never treats a
UUID-shaped IdP subject as an account ID and never reads an owner from tool arguments. An unmapped
identity gets an `isError` result with the code `unauthenticated` before any link is read.

The tool requires `links:read`. It takes `{}` or `{"after":"<last link UUID>"}`, sorts by link
UUID and returns at most 100 links per page:

```json
{
  "links": [{
    "id": "0199ba71-6000-7000-8000-000000000001",
    "peerApp": "beta",
    "sends": ["alpha.activity.completed"],
    "receives": ["beta.activity.completed"],
    "status": "active",
    "createdAt": "2026-10-06T09:00:00Z",
    "updatedAt": "2026-10-06T09:00:00Z"
  }],
  "nextCursor": null
}
```

`sends` and `receives` are event scopes from this product's side. `status` is `active`,
`needs_attention` or `revoked`. Results hold no tokens, secrets or remote subjects. Each page loads
the owner's full link list through the existing suite read, so paging bounds the response but not
the query, and there is no snapshot across concurrent changes. Add `SuiteLinksTool::definition()`
to the definitions that `mcp-tools` and `tool_drift` use, then re-export `backend/mcp-tools.json`.
The [suite guide](suite-and-platform.md) explains links.

## Reference

- [baukit-mcp README](../../rust/crates/baukit-mcp/README.md)
- [baukit-auth README, personal access tokens](../../rust/crates/baukit-auth/README.md#personal-access-tokens)
- [Template MCP doc: add a tool, errors, resources and prompts](../../templates/mcp-remote/docs/remote-mcp.md)
- [ADR 0019: remote MCP only](../adr/0019-remote-mcp-only.md)
- [ADR 0017: identity providers and bearer verification](../adr/0017-identity-providers-and-bearer-verification.md)
- [ADR 0015: request admission](../adr/0015-request-admission.md)
- [Chart values](../../deploy/chart/baukit-app/values.yaml)
- [The auth guide](auth.md)
- [The suite and platform guide](suite-and-platform.md)
