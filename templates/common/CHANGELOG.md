# Changelog

## [Unreleased]

{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
- Added error-response rules to the generated OpenAPI document. Every operation now documents the 400, {% if context.auth_oidc %}401, {% endif %}404, 413, 415, 422, {% if context.auth_oidc %}429, {% endif %}500, and 504 responses its middleware can return, with `X-Request-Id` on every response.
{% if context.quality_strict %}- Added `quality.openapi_compatibility` (`off`, `report`, or `enforce`) to the strict quality gate. It compares `backend/openapi.json` with the base revision and reads accepted breaks from `docs/openapi-accepted-breaks.json`.
{% endif %}{% endif %}{% if context.mcp %}- Added the opt-in MCP stdio package with explicit tool registries, bearer-token providers, and OpenAPI route checks.
{% endif %}{% if context.mobile %}- Added mobile tests for the theme mode control, record store seams, and the route heading focus hook, with `@testing-library/react-native` and `test-renderer` as dev dependencies.
{% if context.auth_oidc %}- Changed the auth mobile app to use the base Jest config, so `test:coverage` now enforces the 70% statement, branch, function, and line floors. The app also gains the `setup` and `test:coverage` scripts that the generated CI already calls. New tests cover the OIDC auth hook, the authenticated local-data provider, and the authenticated API runtime.
{% endif %}{% endif %}{% if context.web and context.auth_oidc %}- Added `E2E_WEB_PORT` to the web Keycloak stack test. Its global setup adds the chosen origin to the realm's web client through the admin API when the client lacks it, and `E2E_KEYCLOAK_WEB_CLIENT_ID` names that client.
- Changed the web Keycloak stack test to import `keycloakStack`, `createKeycloakTestUser`, `signInWithKeycloak`, and `allowKeycloakWebOrigin` from `@baukit/auth-node/keycloak-testing`, now a web dev dependency. `e2e/stack/keycloak.ts` keeps only this product's defaults and exports them as `stack`; the helpers take `stack` explicitly and no longer take a Playwright request context.
- Added auth web tests for the OIDC client wiring, the API parsers and default transport, the authenticated API runtime, and the local-data hook, so `test:coverage` passes its 70% floors.
{% endif %}{% if context.backend and context.auth_oidc %}- Added a Redis service to `compose.yaml` on `127.0.0.1:{{ context.redis_host_port }}`. The API's rate limiter needs it at startup, and `make dev` now starts it.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
