# Changelog

## [Unreleased]

{% if context.mcp %}- Added the opt-in MCP stdio package with explicit tool registries, bearer-token providers, and OpenAPI route checks.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
