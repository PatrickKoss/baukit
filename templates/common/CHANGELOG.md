# Changelog

## [Unreleased]

{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
- Added error-response rules to the generated OpenAPI document. Every operation now documents the 400, {% if context.auth_oidc %}401, {% endif %}404, 413, 415, 422, {% if context.auth_oidc %}429, {% endif %}500, and 504 responses its middleware can return, with `X-Request-Id` on every response.
{% if context.quality_strict %}- Added `quality.openapi_compatibility` (`off`, `report`, or `enforce`) to the strict quality gate. It compares `backend/openapi.json` with the base revision and reads accepted breaks from `docs/openapi-accepted-breaks.json`.
{% endif %}{% endif %}{% if context.mcp %}- Added the opt-in MCP stdio package with explicit tool registries, bearer-token providers, and OpenAPI route checks.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
