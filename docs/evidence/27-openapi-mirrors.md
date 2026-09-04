# 27. Raw OpenAPI mirrors

## Source product files

- `/home/patrick/projects/tiefgang/scripts/openapi-client.sh`
- `/home/patrick/projects/tiefgang/extension/scripts/copy-openapi.mjs`
- `/home/patrick/projects/tiefgang/extension/scripts/check-openapi.mjs`
- `/home/patrick/projects/tiefgang/extension/test/contracts.test.ts`
- `/home/patrick/projects/tiefgang/scripts/quality-gate.sh`
- `/home/patrick/projects/tiefgang/mcp/openapi.json`
- `/home/patrick/projects/tiefgang/extension/openapi.json`

## Observed failure or repeated glue

Tiefgang separately copies `backend/openapi.json` into MCP and extension packages and checks both copies for byte drift. Its `openapi.consumers` list correctly contains TypeScript declaration outputs only.

## Baukit owner

A future Baukit CLI and strict template would own manifest parsing, root containment, byte copying, doctor checks, and strict drift checks. Implementation is deferred.

## Public types and errors

The proposed manifest addition is optional `openapi.mirrors: Vec<String>`. A separate `baukit generate openapi-mirrors` command would copy bytes. Doctor errors distinguish missing, escaped, duplicate, overlapping, unreadable, and differing mirror paths without printing file contents.

## Product-owned inputs

Products own the canonical schema, mirror destinations, declaration consumers, package layout, package inclusion tests, and the reason runtime code needs raw JSON.

## Concurrency, failure, privacy, and cleanup cases

Validation rejects absolute paths, parent traversal, symlink escape, duplicates, and overlap with declaration outputs. The command validates all paths before atomic sibling-file replacement and removes temporary files after failure. Concurrent successful writers produce the same bytes. Checks expose paths but never schema contents.

## Supported runtimes

The future CLI would support the same local and CI platforms as `baukit doctor`. Mirrors may be consumed by Node packages, browser extensions, or other packaged artifacts.

## Product adoption change

Tiefgang could delete the MCP copy line in `scripts/openapi-client.sh`, `extension/scripts/copy-openapi.mjs`, `extension/scripts/check-openapi.mjs`, and duplicate checks in `scripts/quality-gate.sh` and `extension/test/contracts.test.ts`. Package inclusion tests remain.

## Implementation gate

No second raw-schema consumer exists in the audited products. Implementation starts only when another product has a committed raw copy read by a built, tested, or published package and can name the local copy and drift code that adoption will delete.
