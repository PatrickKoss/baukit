# 29. Revisioned write queue and durable form drafts

## Source product files

- `/home/patrick/projects/leitbild/web/src/autosave.ts`
- `/home/patrick/projects/leitbild/web/src/autosave.test.tsx`
- `/home/patrick/projects/leitbild/mobile/src/autosave.ts`
- `/home/patrick/projects/leitbild/mobile/src/autosave.test.ts`
- `/home/patrick/projects/leitbild/mobile/src/screens/program-run-screen.tsx`
- `/home/patrick/projects/redemut/web/src/form-draft.tsx`
- `/home/patrick/projects/redemut/web/src/form-draft.test.ts`
- `/home/patrick/projects/redemut/web/src/onboarding-draft.ts`
- `/home/patrick/projects/redemut/web/src/onboarding.tsx`
- `/home/patrick/projects/redemut/web/test/onboarding-draft.test.ts`
- `/home/patrick/projects/redemut/packages/data/src/form-draft-store.ts`
- `/home/patrick/projects/redemut/packages/data/src/dexie/form-draft-store.ts`
- `/home/patrick/projects/redemut/packages/data/test/form-draft-store.test.ts`

## Observed failure or repeated glue

Leitbild implements acknowledged-revision serialization twice. The web hook coalesces through current refs and exposes conflicts, while mobile keeps a promise tail per section. Neither implementation fences identity or document switches. Redemut separately implements debounced durable drafts over SQLite and Dexie. Its generic hook casts decoded JSON to `T`, and its onboarding flow ignores failed deletion, so a completed draft can reappear.

## Baukit owner

`@baukit/data-contracts` should own two opt-in, framework-free exports. One owns revisioned write serialization, coalescing, cancellation, stale-completion fencing, acknowledged revision, conflict pause, and reset. The other owns versioned draft decoding, dirty and persistence state, save, clear, and scope changes. Existing `KeyValueStore` and scoped persistence remain the storage and identity foundations.

## Public types and errors

The accepted sketch names `RevisionedWriteQueue`, `RevisionedWriteQueueOptions`, `RevisionedWriteScope`, `RevisionedWriteResult`, `RevisionedWriteStatus`, `DurableDraft`, `DraftScope`, `DraftCodec`, `DraftPersistenceOperation`, and `DraftPersistenceError`. The write callback returns a product-mapped conflict as data. Other write failures remain as their original cause in state. Draft persistence wraps read, write, and delete failures with code `draft_persistence_failed` and the operation, without stored content or keys in its message.

## Product-owned inputs

Products supply identity and document keys, initial and edited values, revision types, request mapping, conflict decoding and decisions, draft key encoding, codecs and migrations, storage adapters, debounce timing, retry policy for unknown server outcomes, lifecycle events, schemas, UI, copy, and submission behavior.

## Concurrency, failure, privacy, and cleanup cases

Tests must cover edits during save, three or more queued saves, coalescing, failure and retry, conflict with queued work, cancellation, unmount, account switch, document switch, stale reads and writes, corrupt drafts, schema upgrade, unsupported versions, submit success, and failed deletion. Old-scope completions cannot publish or start queued work. Errors and debug output must omit draft values, document keys, identities, request bodies, and decoder text. Draft cleanup reports deletion failure instead of treating it as success.

## Supported runtimes

The root helpers target the same ES2022 browser, Node, and React Native environments as `@baukit/data-contracts`. They import no React, storage provider, or network client. Hosts inject timers, `AbortController` support, write callbacks, and storage.

## Product adoption change

Leitbild can delete `mobile/src/autosave.ts` and its test. Its web `autosave.ts` can shrink to a product React wrapper that maps section fields and conflict choices to the shared queue. Redemut can delete `FormDraftController`, `storeDraft`, and `readStoredDraft` from `web/src/form-draft.tsx`; its hook remains as product composition. The dedicated SQLite and Dexie draft stores can remain as compatibility adapters until Redemut chooses a data migration.

## Throwaway experiments

None. The study used source inspection only.
