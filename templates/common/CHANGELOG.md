# Changelog

## [Unreleased]

{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
{% endif %}{% if context.mcp %}- Added the opt-in MCP stdio package with explicit tool registries, bearer-token providers, and OpenAPI route checks.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
