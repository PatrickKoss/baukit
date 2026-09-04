# MCP capability

`baukit new NAME --backend --mcp` adds an isolated `mcp/` TypeScript package. MCP needs the generated backend because its route allowlist and declaration both use the committed backend OpenAPI document. Projects without `--mcp` have no MCP files, dependencies, lockfile, manifest entry, or CI job.

## Manifest and authentication

The generator records the selected mode in `baukit.toml`:

```toml
[capabilities]
backend = true
mcp = { authentication = "personal-token" }

[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts", "mcp/src/api/schema.d.ts"]
```

Personal tokens are the default. The stdio bootstrap injects a provider that reads the generated product's `*_API_TOKEN` environment variable. `--mcp-auth caller-supplied` instead documents and loads a module that exports `bearerToken`. Library callers can pass any `BearerTokenProvider` to `createServer` without using the generated bootstrap.

When `--auth oidc` and `--mcp` are selected together, MCP defaults to `node-oidc` and adds `@baukit/auth-node`. Its device-flow client owns discovery, PKCE, refresh, and the locked token cache. `--mcp-auth node-oidc` is rejected unless the OIDC capability is present. Products still own issuer, client ID, scopes, audience, presentation text, and environment variable names.

Existing manifests remain valid because `capabilities.mcp` is optional. To adopt MCP in an existing generated product, add the inline capability, add `mcp/src/api/schema.d.ts` to `openapi.consumers`, copy the current `mcp/` template, and regenerate lockfiles and declarations. No raw OpenAPI file belongs under `mcp/`.

## Package contract

The package has separate read and write registries. Each tool declares `readOnlyHint`, `destructiveHint`, `idempotentHint`, and `openWorldHint`. The initial registry has one read-only example for `GET /items` and no write tools. Products replace the example and author their tool names, descriptions, schemas, scopes, and route choices.

`mcp/src/tool-routes.ts` is the product-owned allowlist. `pnpm openapi:check` compares every registry entry with the allowlist and rejects a path or method absent from the backend schema. TypeScript also restricts allowlist entries to generated OpenAPI paths and methods. The check does not create a tool for each operation.

`pnpm run docs` imports registry metadata from the built package and writes `mcp/docs/tools.md`. `pnpm docs:check` compares the same output without changing the file. Neither command scans source strings.

The API client accepts an injected fetch implementation and bearer-token provider. It limits successful JSON responses to 1 MiB. Authentication failures, network failures, invalid JSON, oversized bodies, and non-success status codes become fixed MCP errors. Backend bodies and exception messages do not cross the tool boundary.

## Stdio and shutdown

Running `node dist/cli.js` starts `StdioServerTransport`. stdout is reserved for JSON-RPC messages. Tool logs contain only the tool name, outcome, and HTTP status when present. Startup and shutdown logs contain the transport or signal, and all logs go to stderr.

`SIGINT` and `SIGTERM` close the MCP server and transport once. `--help` prints usage without starting the transport. The generated tests use the SDK's linked in-memory transport for tool cases and a child-process client for malformed JSON, protocol-clean stdout, credential redaction, and graceful shutdown.

## Generated checks

Run the package gate from `mcp/`:

```sh
corepack pnpm@11.18.0 install --frozen-lockfile
corepack pnpm@11.18.0 build
corepack pnpm@11.18.0 typecheck
corepack pnpm@11.18.0 lint
corepack pnpm@11.18.0 test
corepack pnpm@11.18.0 openapi:check
corepack pnpm@11.18.0 docs:check
```

Generated CI runs this gate only when MCP is selected. `make mcp-fixture-gate` creates a backend plus MCP fixture and checks both packages.

## Product boundaries

The generator does not add product tool names, one tool per OpenAPI operation, destructive defaults, product scopes, consent language, domain recovery text, or mappings for raw backend exceptions. Products own those decisions and keep them when replacing a local MCP bootstrap.
