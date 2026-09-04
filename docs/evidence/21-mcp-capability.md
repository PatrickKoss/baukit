# Evidence for item 21: MCP capability

## Source product files

- `/home/patrick/projects/tiefgang/mcp/src/index.ts`, `server.ts`, `api/client.ts`, `tools/read.ts`, and `tools/write.ts`
- `/home/patrick/projects/leitbild/mcp/src/index.ts`, `server.ts`, `api/client.ts`, `tools/read.ts`, and `tools/write.ts`
- `/home/patrick/projects/eigenruhe/mcp/src/index.ts`, `server.ts`, `api/client.ts`, `tools/read.ts`, and `tools/write.ts`
- `/home/patrick/projects/redemut/packages/mcp-server/src/transports/stdio.ts`, `transports/http.ts`, and `server.ts`

## Observed failure or repeated glue

Tiefgang, Leitbild, and Eigenruhe repeat the package, stdio, API client, server, and split registry layout. Their tools and routes differ, but the bootstrap and failure handling do not. Redemut confirms the same need. Its HTTP transport builds public metadata from request headers and its server returns raw exception messages for unknown API failures. The generated capability omits HTTP and never returns backend exception text.

## Baukit owner

The CLI and `templates/mcp/` own opt-in composition, stdio lifecycle, typed API and token-provider seams, fixed error conversion, registry structure, route and docs checks, lockfile generation, doctor checks, and generated CI.

## Public types and errors

Generated code exposes `BearerTokenProvider`, `ApiClient`, `ApiClientOptions`, `OpenApiRoute`, `AuthenticationError`, `ApiClientError`, `ToolDefinition`, `RequiredToolAnnotations`, `ToolLogger`, `ServerOptions`, and `createServer`. MCP errors use `authentication_failed`, `api_error`, or `tool_error` with fixed messages and an optional HTTP status.

## Product-owned inputs

Products own tool names, descriptions, schemas, annotations, routes, handlers, API URL, token source, OIDC settings, scopes, audience, user instructions, and any recovery or consent text.

## Cases

- Concurrency: the stdio bootstrap closes once when signals race. `@baukit/auth-node` owns refresh and cache locking in `node-oidc` mode.
- Failure: tests cover malformed JSON-RPC, invalid tool input, missing credentials, backend status errors, OpenAPI drift, docs drift, response bounds, and shutdown.
- Privacy: stdout contains protocol messages only. Logs omit arguments, bodies, credentials, and raw exceptions.
- Cleanup: `SIGINT` and `SIGTERM` close the server and transport. The generated package creates no schema copies or token cache in personal-token mode.

## Supported runtimes

Node 24 or later, TypeScript 5.9.3, MCP SDK 1.30.0, and stdio clients compatible with the SDK's supported protocol versions. `node-oidc` uses the generated Node 24 contract of `@baukit/auth-node`.

## Product adoption change

Tiefgang can delete `mcp/src/index.ts` and `mcp/src/server.ts` after it adopts the generated bootstrap and keeps its product-authored API client and tools. Leitbild and Eigenruhe can make the same two-file replacement. Their local auth files become deletable when they also adopt `@baukit/auth-node`. Product repositories are read-only in this batch, so the required released-version adoption remains open.

## Implementation report

### 1. Summary

The CLI now accepts `--mcp` for backend projects and records an optional inline `capabilities.mcp` manifest entry. Personal tokens are the default. OIDC projects default to `node-oidc`, and callers can select an injected module provider. Old manifests still parse without an MCP entry.

The generated package uses MCP SDK 1.30.0, the newest version pinned by the source products. It has a typed OpenAPI client, bounded response reads, fixed errors, separate read and write registries, required annotations, a product-owned route allowlist, registry-derived docs, stdio lifecycle handling, and outcome-only logs. CI, doctor, lockfile generation, OpenAPI consumers, snapshots, and fixture gates include MCP only when selected.

There is no package-design deviation from item 21. The completion criterion that one released product adopt the package remains open because the task made product repositories read-only.

### 2. Files added or changed

- `.github/workflows/ci.yml`
- `CLAUDE.md`
- `Makefile`
- `cli/examples/bless_snapshots.rs`
- `cli/src/lib.rs`
- `cli/src/main.rs`
- `cli/tests/generator.rs`
- `cli/tests/snapshots/mcp.tree`
- `templates/backend/baukit.toml`
- `templates/common/.github/workflows/ci.yml`
- `templates/common/CHANGELOG.md`
- `templates/common/CLAUDE.md`
- `templates/common/scripts/lockfiles.sh`
- `templates/mcp/mcp/.gitignore`
- `templates/mcp/mcp/.prettierignore`
- `templates/mcp/mcp/.prettierrc.json`
- `templates/mcp/mcp/README.md.jinja`
- `templates/mcp/mcp/docs/tools.md`
- `templates/mcp/mcp/eslint.config.js`
- `templates/mcp/mcp/package.json.jinja`
- `templates/mcp/mcp/scripts/check-openapi-allowlist.mjs`
- `templates/mcp/mcp/scripts/generate-tool-docs.mjs`
- `templates/mcp/mcp/src/api/client.ts.jinja`
- `templates/mcp/mcp/src/auth.ts.jinja`
- `templates/mcp/mcp/src/cli.ts.jinja`
- `templates/mcp/mcp/src/server.ts.jinja`
- `templates/mcp/mcp/src/tool-routes.ts`
- `templates/mcp/mcp/src/tools/read.ts`
- `templates/mcp/mcp/src/tools/registry.ts`
- `templates/mcp/mcp/src/tools/write.ts`
- `templates/mcp/mcp/test/server.test.ts`
- `templates/mcp/mcp/test/stdio.test.ts.jinja`
- `templates/mcp/mcp/tsconfig.build.json`
- `templates/mcp/mcp/tsconfig.json`
- `templates/mcp/mcp/vitest.config.ts`
- `docs/platform/mcp-capability.md`
- `docs/evidence/21-mcp-capability.md`

### 3. Verification

- `make cli-ci`: passed after the implementation changes. Rustfmt, clippy with `-D warnings`, and check passed. Tests passed: 9 unit and 37 generator tests, 46 total, with 0 failed and 0 ignored.
- `cargo test --manifest-path cli/Cargo.toml --test generator mcp -- --nocapture`: passed, 5 passed, 0 failed, 32 filtered out.
- `cargo run --manifest-path Cargo.toml --example bless_snapshots` from `cli/`: initially passed and blessed eight trees. A final workspace run failed because another task created `templates/common/__strict__/scripts/__pycache__/check-markdown-links.cpython-312.pyc`. The current tree was copied outside the repository without that foreign cache; the same command passed and blessed all eight trees. The MCP snapshot matches that clean result.
- Clean-copy CLI verification against the current sources: rustfmt, clippy with `-D warnings`, and cargo check passed. `cargo test --manifest-path Cargo.toml` passed 9 unit and 37 generator tests, 46 total. The first clean-copy test run failed 7 of 37 generator tests because the copy lacked the repository `rust/` path; rerunning with a read-only symlink to the workspace Rust directory passed.
- `make mcp-fixture-gate`: final run passed. Generated backend fmt and clippy passed; backend tests passed 9 and ignored 1 PostgreSQL test; the explicit OpenAPI drift test passed 1. The MCP install, build, typecheck, lint, OpenAPI check, and docs check passed; Vitest passed 9 tests in 2 files.
- Docker-gated suite: `CARGO_TARGET_DIR=/tmp/baukit-mcp-docker-target cargo test --manifest-path /tmp/tmp.gKBmZ96Qzh/auth-mcp/backend/Cargo.toml --test postgres_integration -- --include-ignored` passed, 1 passed, 0 failed, 0 ignored.
- Auth fixture `baukit new auth-mcp --backend --mobile --web --auth oidc --mcp`: generated `node-oidc` with `@baukit/auth-node`; install, build, typecheck, lint, test, OpenAPI check, and docs check passed. Vitest passed 9 tests in 2 files. Its first lint run found one Prettier mismatch in generated `auth.ts`; the corrected template passed the full rerun.
- Caller-supplied fixture: install, build, typecheck, lint, and test passed, with 9 tests in 2 files. Its first test run passed 8 and timed out 1 because the stdio test did not inject a caller module; the corrected test passed the rerun.
- Plain `--backend` fixture: targeted `rg` found no MCP package, dependency, manifest entry, workflow job, lockfile command, or generated-project instruction. A whole-tree `rg` found only the pre-existing generic mention of an MCP server in `docs/openapi-drift.md`.
- YAML parsing with Python `yaml.safe_load`: passed for the repository workflow and generated MCP workflow. The preceding Ruby parser attempt failed because Ruby is not installed.
- `scripts/check-version-coherence.py`: passed for 16 crates, the CLI, 18 packages, and 2 charts.
- `python3 deploy/observability/lint/check-metric-names.py`: passed for 1 dashboard, 2 rule files, and 9 local recording rules.
- `git diff --check`: passed.
- `make ci`: failed at the CLI generator suite with 30 passed and 7 failed. The foreign Python bytecode cache broke strict-template generation, and the concurrent auth template snapshot was not yet re-blessed. The TypeScript and Rust stages before `cli-ci` completed. MCP-focused tests and the clean-copy full CLI suite passed afterward.
- One formatting command used `cli/Cargo.toml` while already inside `cli/` and failed because that relative manifest did not exist. The corrected command, `cargo fmt --manifest-path Cargo.toml --all -- --check`, passed.

### 4. Failures in other agents' areas

`templates/common/__strict__/scripts/__pycache__/check-markdown-links.cpython-312.pyc` is an ignored binary created by another task's Python test. It makes embedded-template rebuilds and snapshot blessing fail. The plan log already assigns a generator-side `__pycache__` and `*.pyc` exclusion to the final verification pass. The auth snapshot also changed while the Keycloak-theme task was active; the orchestrator planned a batch-end re-bless.

### 5. Leftovers and open questions

The orchestrator must remove or exclude the foreign Python cache, re-bless all snapshots after concurrent template work settles, and rerun `make ci`. Product adoption against a released version is still required before marking item 21 complete. No dependency-deny or MSRV-specific run was needed because this item added no Rust dependency or new Rust language feature.

### 6. Product adoption note

Tiefgang, Leitbild, and Eigenruhe can delete `mcp/src/index.ts` and `mcp/src/server.ts` after adopting the generated bootstrap while retaining their tool and route code. Each can also delete `mcp/src/auth.ts` when it selects `node-oidc` and moves only product OIDC configuration and presentation callbacks into composition.
