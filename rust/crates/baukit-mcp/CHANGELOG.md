# Changelog

## [Unreleased]

- Seed effective MCP scopes from verified API-token grants. Policies can only narrow
  them; OIDC scopes and `baukit-auth::Principal::scopes()` keep their existing behavior.
- Behavior change: a product that already routes API tokens to `/mcp` grants each token the MCP
  tools its stored grants match after upgrading. Before, every such token had no scopes. Check
  that issued grants use MCP scope names only where tool access is intended.
- Reject API-token principals in Keycloak introspection without forwarding their secrets.
  Document PAT verifier wiring, subject mapping and per-request revocation checks.

## [0.11.0] - 2026-10-22

- Return HTTP 503 with the flat `{"error":"identity_unavailable"}` body and
  `Retry-After: 5` for temporary identity-verification failures, including OIDC
  body-read failures and provider 408, 429, or 5xx responses. Fully received but
  invalid provider metadata or JWKS documents, other provider 4xx responses, and
  server configuration errors return HTTP 500 with
  `{"error":"identity_misconfigured"}` and no `Retry-After`. Both responses omit
  the OAuth challenge and log a safe cause at error level. Invalid credentials
  still return 401.
- Add the optional `suite` feature and `McpServices::with_suite_links` to register `list_suite_links`. It requires `links:read`, resolves the admitted product owner through `SuiteOwnerResolver`, reads through `SuiteLinksRead`, and returns credential-free link metadata in pages of 100.
- `SuiteLinksTool::new` and `McpServices::with_suite_links` require an explicit owner resolver. Unmapped identities receive an `unauthenticated` tool error before links are read. This changes the constructor signatures introduced in this unreleased change.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

- Breaking: `ToolService::call` and `ResourceService::read` take a `CancellationToken` as their final argument. The token signals client cancellation and HTTP disconnects. Cancellation notifications apply only to active requests from the same issuer, subject and OAuth client. Concurrent calls from that caller must use distinct request IDs.
- Breaking: `ScopedTool` requires an `annotations` field. Use `ToolAnnotations::default()` to keep the read/write defaults. Tools can override each hint and title. Drift exports include the overrides.
- Let `McpServices` set the product server name, version, title and initialize instructions.
- Add a server success text prefix without changing structured content or errors.

## [0.9.0] - 2026-10-07

- Validate internal JWKS overrides separately from the public issuer. Reject unknown config fields and correct migration scope instructions.

- Add product-defined structured and text-only tool errors. Use `ToolError::new` instead of struct literals for the default error.
- Add optional scoped resource and prompt services, scope-filtered discovery, safe errors, and committed definition exports. The router takes `McpServices` with tools required and resources and prompts optional.

- Check tool, resource and prompt scopes in the HTTP authorizer. Report denied requests as 403 in request metrics.
- Accept a shared IdentityVerifier and configured authorization-server metadata. Keep token binding in the supplied verifier.

## [0.8.0] - 2026-10-07

- Add an authentication policy port with effective principals and scopes, typed denials, and policy metrics. Add fail-closed Keycloak introspection with bounded token-hash caching and timeout. The router now requires an explicit policy.

- Add OAuth-protected Streamable HTTP mounting, exact Host and Origin checks, scoped tool service ports, discovery metadata, body limits, rate limits, and request metrics.
