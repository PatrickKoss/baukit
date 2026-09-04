# Evidence for other deferred contracts

## Source product files

The product revisions examined were Tiefgang `861cf0a994d5e63ec245e645023c80575759c191`, Leitbild `25eda071f0e2538b78a3ea62129a73770d506e2b`, Redemut `b4e8a9872595260d3f26af7d8d085aac98485e51`, and Eigenruhe `36b468d015f4aebd83a11bd662c7ff82124711fb`.

| Candidate | Main files inspected |
| --- | --- |
| Content bundles and build tools | Redemut `backend/crates/redemut-content-compiler/src/lib.rs`, `scripts/build-bundled-content.mjs`; Eigenruhe `content/tools/manifest-schema.ts`, `build-manifest.ts`, `build-clips.ts`, `tts/provider.ts` |
| Private artifacts | Redemut `packages/data/src/spoken-attempt-store.ts`, `packages/data/src/dexie/spoken-attempt-store.ts`, `mobile/src/spoken-storage.ts`; Eigenruhe `mobile/src/downloads/platform-file.native.ts` |
| Celebrations | Redemut `packages/domain/src/celebrations/orchestrator.ts`; Eigenruhe `mobile/src/features/celebrations/orchestrator.ts`; [Baukit ADR 0001](../adr/0001-product-experience-package-boundaries.md) |
| Context menus and dialogs | Redemut `packages/ui/src/context-menu.tsx`, `packages/ui/test/components.test.tsx`, `web/src/confirm-dialog.tsx`; Tiefgang `mobile/src/components/context-menu.tsx`, `mobile/src/navigation/context-menu.test.tsx`; Eigenruhe `mobile/src/components/context-menu.tsx`; Leitbild `web/src/confirm-dialog.tsx`, `confirm-dialog.test.tsx` |
| Secret URL tokens | Redemut `backend/crates/redemut-services/src/lib.rs`, `backend/crates/redemut-api/src/lib.rs`, `backend/migrations/20260829193048_calendar_feed_tokens.sql` |
| SQLite migrations | Redemut `packages/data/src/migrations.ts`; Tiefgang `mobile/src/db/sqlite/migrations.ts`, `migrations.test.ts`; Eigenruhe `mobile/src/db/sqlite/migrations.ts`, `migrations.test.ts` |
| Feature gates and extensions | Redemut `docs/adr/feature-gates.md`, `backend/crates/redemut-bin/src/application.rs`, `web/src/ready-app.tsx`, `packages/domain/src/features.ts`; Tiefgang `extension/manifest.json`, `extension/src/sync.ts`, `extension/src/sync.test.ts` |
| Sync | Eigenruhe `mobile/src/sync/engine.ts`, `provider.tsx`; Tiefgang `mobile/src/sync/engine.ts`, `provider.tsx`; Redemut `packages/sync/src/sync-engine.ts` |
| LLM and speech | Leitbild `backend/crates/leitbild-ai/src/lib.rs`, `backend/crates/leitbild-ports/src/lib.rs`; Redemut `backend/crates/redemut-elevenlabs/src/lib.rs`, `backend/crates/redemut-llm-coach/src/lib.rs`; Eigenruhe `content/tools/tts/provider.ts` |
| Runtime images | Each product's `backend/Dockerfile`; Tiefgang, Redemut, and Eigenruhe `.github/workflows/ci.yml`; Eigenruhe `Makefile`; Leitbild `deploy/Dockerfile` |
| MCP | Tiefgang `mcp/src/tools/read.ts`, `write.ts`; Leitbild `mcp/src/tools/shared.ts`; Eigenruhe `mcp/src/tools/read.ts`, `write.ts`; Redemut `packages/mcp-server/src/server.ts` |
| Network-first cache | Leitbild `mobile/src/record-store.ts`, `repositories.ts`, `repositories.test.ts` |
| Markdown and word count | Leitbild `web/src/editor/markdown-commands.ts`, `markdown-shortcuts.ts`, `markdown-commands.test.ts`, `backend/crates/leitbild-domain/src/journal.rs`, `backend/content/fixtures/word-count.json`, `web/src/insights/text.ts`, `web/src/privacy.ts`; Tiefgang `mobile/src/integrations/files/markdown.ts` |

## Observed failure or repeated glue

SQLite runners repeat version ordering and transactional application with incompatible version stores. Three sync engines repeat orchestration but expose different callbacks and protocol rules. Four MCP servers repeat JSON-text results, safe `isError` mapping, and bounded outcome logging. Product Dockerfiles repeat the generated reproducible distroless target pattern. The other rows still lack their stated second consumer, matching behavior, settled product rule, or required Fitness Tracker comparison.

## Baukit owner

Potential owners remain the packages named in the plan: data-contract conformance for SQLite and private artifacts, `@baukit/sync-client` conformance for repeated sync invariants, `@baukit/a11y-core` for proven headless interactions, auth documentation for capability links, the generated backend Dockerfile for minimal images, and an optional MCP runtime for small safe execution helpers. This study assigns no new owner where evidence is absent.

## Public types and errors

No public types or errors are proposed by this deferred study. A later SQLite study should define only a test adapter and conformance failures. A later MCP study may define request metadata and safe execution results, but must first settle structured-output compatibility. The sync comparison supports assertions, not a coordinator type.

## Product-owned inputs

All domain schemas, content metadata, retention durations, UI and copy, feature names, routes, permissions, SQL, sync payloads, provider requests, prompts, image contents, tool schemas, cache keys, Markdown dialects, word-count definitions, and dialog styling remain product inputs.

## Concurrency, failure, privacy, and cleanup cases

Future studies must cover atomic install and interruption, private-file erasure, one-shot queue persistence, menu and dialog focus after failure or routing, capability-link leakage, migration rollback and restart, sync single-flight and identity changes, build cache corruption, provider timeout and secret handling, non-root image execution, bounded MCP errors and logs, cache invalidation and empty-cache behavior, editor selection edges, and fixture parity at every word-count ingress.

## Supported runtimes

The evidence spans Rust services and Linux containers, modern browsers, Node MCP processes, Expo web, iOS, Android, Dexie, and Expo SQLite. Each future contract must name only the runtimes exercised by its own conformance suite.

## Product adoption change

No product file becomes deletable from this study. SQLite, MCP, and any later headless UI adoption need separate implementation decisions before deletion can be named. Sync engines remain product-owned. The generated distroless Dockerfiles already follow Baukit's template, so there is no new duplicate to remove.

## Throwaway experiments

None. The study used source inspection only.
