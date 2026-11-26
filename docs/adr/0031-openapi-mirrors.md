# ADR 0031: Raw OpenAPI mirrors wait for a second consumer and would be a separate manifest list from declaration consumers

## Status

Deferred, 2026-11-27.

## Context

A product's backend writes one canonical `openapi.json`. The manifest's
`openapi.consumers` lists TypeScript declaration files that
`baukit generate openapi-client` produces from it with `openapi-typescript`.
One consumer product also commits byte copies of the raw schema inside other
packages, such as an MCP server or a browser extension, and keeps them in sync
by hand.

Listing a raw copy under `consumers` would run the declaration generator
against the wrong path, or add path-specific branches to it. Only one product
needs raw copies today, so baukit has not built anything for them.

## Decision

Baukit does not support raw schema mirrors today. `OpenApiPaths` in
`cli/src/manifest.rs` has `schema`, `consumers` and the older `typescript`
fallback, and nothing else.

When a second product needs raw copies, the design is:

- An optional `openapi.mirrors` list, separate from `openapi.consumers`. An
  absent or empty list means no mirrors.
- A mirror holds exactly the schema's bytes. The check compares bytes and does
  not parse JSON.
- Every path is relative to the product root. Validation rejects absolute
  paths, parent components and symlinks that resolve outside the canonicalized
  root, plus duplicates and overlaps with the schema or a consumer.
- `baukit doctor` stays read-only and fails on a missing, escaped or differing
  mirror without printing a content diff.
- A separate `baukit generate openapi-mirrors` command writes each mirror
  through a temporary sibling and a rename, so no destination holds a partial
  file. The strict gate keeps schema drift, declaration drift and mirror drift
  as three separate checks.

## Enforced by

Nothing. `OpenApiPaths` does not set `deny_unknown_fields`, so a manifest that
adds `mirrors` today parses and the key is ignored. Adding
`deny_unknown_fields` to `OpenApiPaths`, with a test in `cli/tests/` that a
`mirrors` key fails to parse, would make the missing feature visible.

## Consequences

The CLI keeps one OpenAPI output type. The product with raw copies keeps its
own sync step and its own drift check until this lands.

## History

- 2026-09-04, `29c73825`: the mirror design was written down and deferred
  until a second product has a raw-schema consumer.
