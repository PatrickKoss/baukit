# Changelog

## [Unreleased]

- Add product-defined structured and text-only tool errors. Use `ToolError::new` instead of struct literals for the default error.
- Add optional scoped resource and prompt services, scope-filtered discovery, safe errors, and committed definition exports. The router takes `McpServices` with tools required and resources and prompts optional.

- Check tool, resource and prompt scopes in the HTTP authorizer. Report denied requests as 403 in request metrics.
- Accept a shared IdentityVerifier and configured authorization-server metadata. Keep token binding in the supplied verifier.

## [0.8.0] - 2026-10-07

- Add an authentication policy port with effective principals and scopes, typed denials, and policy metrics. Add fail-closed Keycloak introspection with bounded token-hash caching and timeout. The router now requires an explicit policy.

- Add OAuth-protected Streamable HTTP mounting, exact Host and Origin checks, scoped tool service ports, discovery metadata, body limits, rate limits, and request metrics.
