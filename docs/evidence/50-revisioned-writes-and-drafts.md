# Revisioned writes and durable drafts evidence

Plan item 14, "Add separate revisioned write and draft helpers".

## Source revisions

- Baukit baseline `f4fa786`.
- Leitbild `bd38b33` and Redemut `a782538`. The files below were read at these revisions.
- [Study 29](../studies/29-write-queue-and-form-drafts.md) supplied the required cases.

## Observed failure

Leitbild has two write queues that disagree. `web/src/autosave.ts` (193 lines) keeps the revision in
a ref, sends one save at a time, and maps `revision_conflict` at `:99` to a paused state with a
"keep mine" action. `mobile/src/autosave.ts` (51 lines) chains a promise tail per section with no
coalescing and no conflict state, so a stale revision error looks like any other failure. Neither
file separates "the server may have applied this" from "the server refused this". Leitbild web
covers that gap in `web/src/write-intent.ts`, which derives an idempotency key from a SHA-256 of
the operation and payload and keeps it in `sessionStorage` until the write succeeds.

Leitbild also has an independent draft store. `web/src/journal-drafts.ts` (402 lines) captures a
lease of `{ generation, subject }` per editor at `:14` and checks it on every read, write, and
remove. Records live in `sessionStorage` under `leitbild:journal-draft:v1:` with a `baseEtag`,
`version: 1`, and a `resolved-new-entry` marker. Reads return missing, ready, or unavailable, and
write and remove return a boolean. `pages/JournalEditorPages.tsx:504-518` removes the draft after a
save only when there are no later changes, and otherwise rewrites it with the new `baseEtag`.

Redemut's `web/src/form-draft.tsx` (344 lines) has `FormDraftController` at `:39`, which chains
writes, restores a failed pending write, and bumps a clear generation. `storeDraft` at `:326` writes
`{ version: 1, value }`, and `readStoredDraft` at `:330` returns the unchecked `value`, which
`useFormDraft` casts with `as T` at `:181`. The SQLite and Dexie stores in
`packages/data/src/form-draft-store.ts` and `packages/data/src/dexie/form-draft-store.ts` expose get,
set, and delete. `web/src/onboarding-draft.ts:104-124` swallows failed saves and failed clears, so a
completed onboarding draft can come back on the next visit.

## Baukit owner

`@baukit/data-contracts` owns two opt-in entry points, `/revisioned-writes` and `/durable-draft`.
Neither is re-exported from the package root, following `/limits`. Neither imports React or the
other helper. Products own debounce, page hide, API mapping, idempotency keys, conflict resolution,
form values, codecs, schema upgrades, recovery copy, and storage adapters.

## Decisions

**Write outcomes.** The write callback returns `accepted` with the new revision, `conflict` with
the server's revision and a product payload, or `rejected` with an error the product knows came
before the server accepted anything. Any throw or rejected promise means the outcome is unknown.
That default is the conservative one: a network error after the request left the client cannot
prove the server refused it.

**Unknown outcome.** The queue moves to `unknown-outcome` and keeps the value and expected revision
in `uncertain`, never merged with newer edits. `flush` does not send in that state. `retry` sends
the same value and revision with `afterUnknownOutcome: true`, so a product can reuse the key it
derives from the payload, as Leitbild's `write-intent.ts` already does. The alternative is to read
the server and `reset`. The queue never re-labels an unknown outcome as a definite failure.

**Definite failure.** `rejected` puts the value back into `unsent`, merged with newer edits through
the coalescer, and pauses as `failed`. `flush` or `retry` resumes. This matches Leitbild web, which
retries after the next edit or an explicit call.

**Conflict.** A conflict pauses the queue and later edits stay in `unsent`. Only `reset` resumes,
because only the product knows which revision to trust. "Keep mine" is `reset` to the conflict's
current revision followed by `enqueue`. This follows study 29's conflict row.

**Coalescing.** The default keeps the newer value, which is what Redemut's controller and Leitbild
web do. A product that sends patches passes `coalesce(older, newer)`. `enqueue` never sends; the
product debounces and calls `flush`. Concurrent `flush` calls share one drain.

**Generation fence.** `reset(next)` bumps a generation, aborts the in-flight write through the
per-generation `AbortSignal` passed to the callback, drops unsent and uncertain state, and ignores
any late result. `cancel` does the same and stays `cancelled` until `reset`. The host `signal`
option calls `cancel` when it aborts.

**Draft state names.** Study 29 listed restored, dirty, saving, failed, corrupt, and
unsupported-version. These describe two independent things, so the snapshot has two fields plus a
flag. `recovery` says what `open` found (`none`, `restored`, `corrupt`, `unsupported-version`, and
`unavailable` for a failed read, taken from Leitbild's read result). `persistence` says what
storage is doing (`loading`, `idle`, `saving`, `clearing`, `failed`). `dirty` compares the local
revision with the one storage holds. A restored draft can be dirty, and a corrupt one can be clearing
for a discard, which one enum could not show.

**Blocked saves.** While recovery is `corrupt` or `unsupported-version`, `save` resolves `blocked`.
`open` never writes or deletes. A newer app version's draft is not overwritten by an older one, and
the product decides whether to discard.

**Codec.** Storage holds `{ version, value }`, the same envelope as Redemut's version 1. `decode`
receives unchecked JSON and returns a tagged result. A version above `codec.version` returns
`unsupported-version` without calling `decode`. A throwing decoder counts as `corrupt`. A value
decoded from an older version counts as upgraded, so the draft opens dirty and the next save
rewrites it.

**Clear.** `clear({ reason: 'submitted', localRevision })` records `submission: 'confirmed'` before
touching storage and keeps the value visible. If the local revision moved on, the stored draft stays
and the call resolves `newer-edits-kept`, which is what Leitbild's editor does by hand. A failed
delete rejects with `DraftPersistenceError` and leaves `persistence: 'failed'` with
`submission: 'confirmed'` in the snapshot. `clear({ reason: 'discarded' })` returns to the initial
value.

## Lease and base revision comparison

| Concern                   | Leitbild `journal-drafts.ts`                                          | Redemut `form-draft.tsx`                           | Baukit `/durable-draft`                                                      |
| ------------------------- | --------------------------------------------------------------------- | -------------------------------------------------- | ---------------------------------------------------------------------------- |
| Scope fence               | Lease `{ generation, subject }` captured per editor, checked per call | `activeRef` and a clear generation inside the hook | One session object per `open`; work of an older session never publishes      |
| Ordering                  | Per-lease save queue                                                  | `#writeChain` per controller                       | One tail for all storage work, so A to B to A reads what A's last write left |
| Server revision           | `baseEtag` stored beside the request                                  | None                                               | Kept inside the product value through the codec                              |
| Read failure              | `unavailable`                                                         | Swallowed, shown as no draft                       | `recovery: 'unavailable'`, `persistence: 'failed'`                           |
| Invalid record            | Wrong shape is missing; invalid JSON is `unavailable`                 | Ignored envelope, unchecked `as T` on the value    | `corrupt`, saves blocked until discard                                       |
| Write and delete failures | Boolean return                                                        | Delete propagates, write restores pending value    | `DraftPersistenceError` with `operation`, kept in the snapshot               |
| Clear after submit        | Removes only when no later changes, else rewrites with new `baseEtag` | `clearAfterSave` clears pending and persisted data | `submitted` with `localRevision`; `newer-edits-kept` when edits followed     |

Leitbild's lease protects many editors sharing one store. Baukit uses one controller per editor
instead, so a product that shares a controller must key its hook on the scope. The base ETag stays a
product field: the fixture keeps `baseRevision` in the draft value and seeds the queue from it after
a restore. A generic `baseRevision` slot would have forced a revision type on products that have
none, such as Redemut.

## Public types and errors

`@baukit/data-contracts/revisioned-writes`: `createRevisionedWriteQueue(options)`,
`RevisionedWriteQueue` (`getSnapshot`, `subscribe`, `enqueue`, `flush`, `retry`, `reset`,
`cancel`), `RevisionedWriteQueueOptions` (`initial`, `write`, `coalesce`, `signal`),
`RevisionedWriteScope` (`scope`, `document`, `acknowledgedRevision`), `RevisionedWriteRequest`
(`scope`, `document`, `value`, `expectedRevision`, `afterUnknownOutcome`, `signal`),
`RevisionedWriteResult`, `RevisionedWriteStatus` (`idle`, `dirty`, `writing`, `failed`,
`unknown-outcome`, `conflict`, `cancelled`), `RevisionedWriteSnapshot` (`status`, `scope`,
`document`, `acknowledgedRevision`, `unsent`, `uncertain`, `conflict`, `error`),
`RevisionedWriteConflict`, and `UncertainRevisionedWrite`.

`@baukit/data-contracts/durable-draft`: `createDurableDraft(options)`, `DurableDraft`
(`getSnapshot`, `subscribe`, `open`, `update`, `save`, `clear`, `close`), `DurableDraftOptions`
(`store`, `key`, `codec`), `DraftCodec`, `DraftDecodeResult`, `DurableDraftSnapshot`,
`OpenDurableDraftSnapshot`, `ClosedDurableDraftSnapshot`, `DraftRecovery`, `DraftPersistence`,
`DraftSubmission`, `DraftClearRequest`, `DraftSaveOutcome` (`saved`, `clean`, `blocked`, `stale`),
`DraftClearOutcome` (`cleared`, `newer-edits-kept`, `stale`), `DraftPersistenceOperation`, and
`DraftPersistenceError` with `code: 'draft_persistence_failed'` and `operation`.
`createDurableDraft` throws `RangeError` when `codec.version` is not a positive safe integer.

## Cases

`src/revisioned-writes.test.ts` has 30 tests: edits during a save, an older acknowledgement never
marking a newer value saved, three queued writes with default and custom coalescing, concurrent
flushes, retry and flush after a definite failure, the coalescer merging a failed value, an unknown
outcome kept apart from newer edits and replayed with the flag, a synchronous throw, a conflict
stopping later writes, resume through `reset`, cancellation with a late result, the injected signal
before and after abort, account and document switches, A to B to A, and snapshot stability.

`src/durable-draft.test.ts` has 45 tests: Redemut's version 1 envelope, edits while loading,
malformed envelopes, a throwing decoder, blocked saves then discard, codec upgrade, unsupported
version with no decode, write, or delete, saving states, edits during a save, queued saves writing
only the newest value, write and quota failures without key or value in the message, read failure,
encoder failure, submitted and discarded clears, a failed delete staying visible, clear waiting for
a write, and scope switches with a late read, write, failure, and delete, including A to B to A and
no write into another account.

`src/revisioned-drafts.fixture.test.ts` is the non-React fixture. It imports both entry points
through the package name and composes them in a plain editor class. It covers the happy path, a
lost response that recovers the draft and then reports a conflict against its own write, and a late
acknowledgement for one account staying out of the next. Study 29 also asked for a generated
fixture. No template uses these helpers yet, so the fixture lives in the package.

## Supported runtimes

Both helpers target ES2022 with no Node, browser, or React Native import. The queue uses the global
`AbortController` and `AbortSignal`, which browsers, Node 24, and React Native provide. The emitted
types name the global `AbortSignal`, so a TypeScript consumer needs the DOM lib, `@types/node`, or
React Native's globals, as with `@baukit/api-runtime`. The draft helper runs on any `KeyValueStore`
adapter.

## Failure behavior

`flush` and `retry` resolve the snapshot for every write outcome and report failures in `status`
and `error`. They reject only when the product's `coalesce` throws. Nothing retries on its own. The draft helper rejects `save` and `clear` with
`DraftPersistenceError` and keeps the failure in the snapshot. `open` resolves after a read failure
and reports `unavailable`. `open` rejects only when the key encoder throws, and then keeps the
current scope. Storage work of a closed or replaced scope that had not started resolves `stale`.

## Privacy boundary

`DraftPersistenceError` messages name only the operation. The storage error is kept as `cause`
after `normalizeStorageError`, and the tests check that neither the key nor draft content appears in
the message. A decoder error is not surfaced at all; the draft reports `corrupt`. The queue passes
product errors through `error` without reading them. Keys should include the identity partition,
and Baukit's scoped persistence lifecycle still owns the store's partition.

## Breaks

None. Both entry points are new, and the package root is unchanged.

## Product adoption change

- Leitbild: replace `mobile/src/autosave.ts` and its tests with the queue. Shrink
  `web/src/autosave.ts` to a hook over the queue that maps `revision_conflict` to a `conflict`
  result and "keep mine" to `reset` plus `enqueue`. Pass the `write-intent.ts` key when
  `afterUnknownOutcome` is true. Move `web/src/journal-drafts.ts` onto the durable draft with a
  `KeyValueStore` over `sessionStorage`, keeping `baseEtag` in the codec value and the
  `resolved-new-entry` marker in the product.
- Redemut: delete `FormDraftController`, `storeDraft`, and `readStoredDraft` from
  `web/src/form-draft.tsx` and give `useFormDraft` a codec that validates each form value. Adapt
  the SQLite and Dexie form-draft stores to `KeyValueStore` so the existing tables stay. Move
  `web/src/onboarding-draft.ts` onto the durable draft so a failed clear is shown instead of
  swallowed. `web/src/dialog-durable-write.ts` stays local; it appends idempotent events rather than
  keeping a draft.

The plan's acceptance needs these deletions. They happen in the adoption pass, not in this Baukit
change.

## Verification

- `corepack pnpm --dir typescript build`, `format:check`, `lint`, `test`, and `check` passed. The
  data-contracts suite ran 234 tests.
- `pnpm pack` of `@baukit/data-contracts`, extracted into an empty Node 24 project, imported
  `/revisioned-writes` and `/durable-draft`, ran one write and one draft save, and confirmed the
  root does not export either helper.
