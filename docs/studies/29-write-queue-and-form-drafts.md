# 29. Revisioned write queue and durable form drafts

## Question and scope

Should Baukit own the framework-free mechanics behind Leitbild's revision-aware server autosave and Redemut's local form-draft persistence? This study treats them as separate concerns. It compares the product code with `@baukit/data-contracts`, `@baukit/sync-client`, `@baukit/preferences-core`, `docs/platform/offline-readiness-contract.md`, and `docs/platform/local-data-ownership-contract.md`. React hooks, request fields, form schemas, conflict choices, and user copy are outside the proposed helpers.

## Evidence table

| Product or Baukit area | File | What it does | What varies |
| --- | --- | --- | --- |
| Leitbild web | `/home/patrick/projects/leitbild/web/src/autosave.ts` | `useSectionAutosave` debounces edits, sends the last acknowledged revision, waits for an active save, preserves edits made during that save, and stops on a typed API conflict. | It is a React hook. It knows section request fields, numeric revisions, and the `revision_conflict` API code. |
| Leitbild web | `/home/patrick/projects/leitbild/web/src/autosave.test.tsx` | Tests debounce, revision advancement, conflict pause, and an explicit "keep mine" retry. | The test drives product copy and React rendering. It does not cover unmount, identity change, document change, cancellation, or several concurrent callers. |
| Leitbild mobile | `/home/patrick/projects/leitbild/mobile/src/autosave.ts` | `AutosaveCoordinator` keeps a promise tail and acknowledged revision for each section. Later saves continue after a failed save. | It sends every queued value rather than coalescing to the newest one. It has no cancellation, conflict state, or scope reset. |
| Leitbild mobile | `/home/patrick/projects/leitbild/mobile/src/autosave.test.ts` | Proves that two saves for one section run in order and use revisions 4 then 5. | It covers one success path only. |
| Leitbild mobile composition | `/home/patrick/projects/leitbild/mobile/src/screens/program-run-screen.tsx` | Creates one coordinator for a program screen and seeds revisions from loaded responses. Each editor owns debounce and save status. | Program loading, locale, cached-mode policy, copy, and React Native lifecycle stay in the product. The coordinator is not reset by `runId`. |
| Redemut web | `/home/patrick/projects/redemut/web/src/form-draft.tsx` | `FormDraftController` debounces and serializes local writes, restores a failed pending write, flushes during navigation and page hide, waits before deletion, and suppresses stale UI callbacks after unmount. `useFormDraft` stores a version 1 envelope. | The hook knows React, a product navigation event, recovery UI, repository names, and unchecked casts to the form type. |
| Redemut storage | `/home/patrick/projects/redemut/packages/data/src/form-draft-store.ts` and `/home/patrick/projects/redemut/packages/data/src/dexie/form-draft-store.ts` | SQLite and Dexie implement the same keyed JSON get, set, and delete operations. | SQL, table layout, Dexie transactions, and the boolean delete result are adapter details. |
| Redemut tests | `/home/patrick/projects/redemut/web/src/form-draft.test.ts` and `/home/patrick/projects/redemut/packages/data/test/form-draft-store.test.ts` | Test coalescing, immediate flush, account-separated keys, submit cleanup, recovery, page-hide flush, discard, and both storage engines. | Corrupt generic envelopes, schema migration, scope-switch races, and failed deletion are not tested. |
| Redemut onboarding | `/home/patrick/projects/redemut/web/src/onboarding-draft.ts`, `/home/patrick/projects/redemut/web/src/onboarding.tsx`, and `/home/patrick/projects/redemut/web/test/onboarding-draft.test.ts` | Stores a versioned localStorage draft, rejects malformed or unknown versions, accepts one legacy field shape, saves every step, and clears after completion. Storage failures do not stop onboarding. | The schema, step model, localStorage key, compatibility rule, and decision to ignore storage failures are product policy. A failed clear can leave a completed draft to reappear. |
| Baukit data contracts | `typescript/packages/data-contracts/src/contracts.ts` and `typescript/packages/data-contracts/README.md` | Provides a JSON `KeyValueStore`, normalized quota and closed-store errors, transactions, schema metadata, and identity-scoped storage lifecycle. | It has no draft decoder, dirty state, write debounce, or per-document revision queue. |
| Baukit sync client | `typescript/packages/sync-client/src/scheduler.ts` and `typescript/packages/sync-client/README.md` | `SyncScheduler` serializes an opaque sync run and can collapse requests into one follow-up run. | It does not carry a document value or acknowledged revision. The offline contract deliberately leaves conflict algorithms and payloads to products. |
| Baukit preferences | `typescript/packages/preferences-core/src/controller.ts` and `typescript/packages/preferences-core/README.md` | Serialized mode orders preference patches, rejects failed writes without stopping later writes, drops queued work on identity switch, and fences stale completions. | It normalizes a fixed preference registry and exposes committed values. It is not a reusable document queue or draft codec. |
| Baukit platform contracts | `docs/platform/offline-readiness-contract.md` and `docs/platform/local-data-ownership-contract.md` | Define pending work, stale response rules, cancellation uncertainty, identity reset order, opaque partitions, and late-work fencing. | They do not define document revisions, autosave requests, draft versions, form schemas, or conflict resolution. |

The shared code should use the existing storage and identity contracts. It should not create another partition registry or turn `SyncScheduler` into a document autosave controller.

## Candidate interface or contract sketch

The write queue and draft helper should be separate exports. The names below are a design sketch, not an implementation commitment.

```ts
type RevisionedWriteResult<TRevision, TConflict> =
  | { readonly kind: "accepted"; readonly revision: TRevision }
  | {
      readonly kind: "conflict";
      readonly currentRevision: TRevision;
      readonly conflict: TConflict;
    };

type RevisionedWriteStatus =
  | "idle"
  | "queued"
  | "writing"
  | "failed"
  | "conflict"
  | "cancelled";

interface RevisionedWriteScope<TScope, TDocument, TRevision> {
  readonly scope: TScope;
  readonly document: TDocument;
  readonly acknowledgedRevision: TRevision;
}

interface RevisionedWriteQueueOptions<TScope, TDocument, TValue, TRevision, TConflict> {
  readonly write: (input: {
    readonly scope: TScope;
    readonly document: TDocument;
    readonly value: TValue;
    readonly expectedRevision: TRevision;
    readonly signal: AbortSignal;
  }) => Promise<RevisionedWriteResult<TRevision, TConflict>>;
  readonly coalesce?: (older: TValue, newer: TValue) => TValue;
}

interface RevisionedWriteQueue<TScope, TDocument, TValue, TRevision, TConflict> {
  getSnapshot(): {
    readonly status: RevisionedWriteStatus;
    readonly acknowledgedRevision: TRevision;
    readonly queuedCount: number;
    readonly conflict: TConflict | null;
    readonly error: unknown;
  };
  enqueue(value: TValue): void;
  flush(): Promise<void>;
  retry(): Promise<void>;
  reset(scope: RevisionedWriteScope<TScope, TDocument, TRevision>): void;
  cancel(): void;
  subscribe(listener: () => void): () => void;
}
```

`reset` increments an internal generation, aborts the old request when possible, drops its queued values, and ignores its late completion. A conflict pauses the queue. The product decides whether to call `reset` with the server revision and retry its value, replace the local value, or open a merge flow. `write` returns conflicts as data so the queue does not parse product API errors. The default coalescer keeps the newest unsent value. A caller can supply a coalescer that preserves operations instead.

```ts
interface DraftScope<TIdentity, TDocument> {
  readonly identity: TIdentity;
  readonly document: TDocument;
}

interface DraftCodec<TValue, TStored extends JsonValue> {
  encode(value: TValue): TStored;
  decode(stored: JsonValue):
    | { readonly kind: "decoded"; readonly value: TValue; readonly needsSave: boolean }
    | { readonly kind: "corrupt" }
    | { readonly kind: "unsupported-version"; readonly version: JsonValue };
}

type DraftPersistenceOperation = "read" | "write" | "delete";

class DraftPersistenceError extends Error {
  readonly code: "draft_persistence_failed";
  readonly operation: DraftPersistenceOperation;
  readonly cause: unknown;
}

interface DurableDraft<TIdentity, TDocument, TValue> {
  open(scope: DraftScope<TIdentity, TDocument>, initial: TValue): Promise<void>;
  getSnapshot(): {
    readonly value: TValue;
    readonly restored: boolean;
    readonly dirty: boolean;
    readonly persistence: "idle" | "pending" | "saving" | "failed";
    readonly recovery: "none" | "corrupt" | "unsupported-version";
    readonly error: DraftPersistenceError | null;
  };
  update(value: TValue): void;
  save(): Promise<void>;
  clear(): Promise<void>;
  reset(value: TValue): void;
  close(): Promise<void>;
  subscribe(listener: () => void): () => void;
}
```

The draft factory should accept the existing `KeyValueStore`, an injected key encoder, a codec, and an optional debounce clock. `open` and `close` fence late work by scope generation. The helper reports corrupt and unsupported data without returning an unchecked `T`. It does not decide whether to discard, migrate, or show recovery copy. `clear` rejects with a delete-specific error so a successful server submission cannot be confused with successful local cleanup.

These interfaces should be framework-free opt-in exports from `@baukit/data-contracts`. They do not share a controller. Their only common inputs are a logical scope and observable state.

## Required-case coverage

| Required case | Coverage today | Required package behavior or missing proof |
| --- | --- | --- |
| Edits during save | Leitbild web compares the saved body with the current body after a request. Redemut can schedule a new pending value while an older write is in its promise chain. | Prove the newer value remains dirty and is sent after the active write without allowing the old completion to mark it saved. |
| Several queued saves | Leitbild mobile tests two ordered saves with successive revisions. Redemut serializes flushes through `#writeChain`. | Test three or more enqueues, default coalescing, a custom non-coalescing policy, and concurrent `flush` callers. Leitbild web does not prove this case. |
| Retry | Leitbild mobile continues its tail after failure. Leitbild web retries after another edit or an explicit call. Redemut restores a failed pending write. | Preserve the same acknowledged revision after transport failure. Retry must not duplicate an accepted write whose outcome is unknown without product guidance. |
| Conflict | Leitbild web tests pause and "keep mine" against the returned current revision. | Prove queued writes stop, no later value is sent against a known-stale revision, and only a product decision resumes or resets the queue. Mobile lacks this state. |
| Unmount | Redemut flushes on cleanup and suppresses callbacks through `activeRef`. Leitbild clears only the debounce timer. | `close` must define whether pending local drafts flush. Queue cancellation must abort where supported and ignore every late completion. |
| Account switch | Redemut draft keys include the account, and Baukit's scoped persistence lifecycle changes the store partition. | Test A to B to A with an old read, write, and delete settling late. Redemut's shared `activeRef` can become true again for a new effect, so its current hook does not prove stale-read fencing. |
| Document switch | Product components usually remount or use React keys. Redemut includes the route in the draft key. | Test a switch while read and write operations are active. Neither product has a controller-level proof. |
| Corrupt draft | Redemut onboarding rejects invalid JSON and malformed records. The generic hook ignores an invalid envelope but has no direct test. | Return a typed recovery state, do not expose decoded source text in errors, and leave discard or migration to the product. |
| Schema upgrade | Redemut onboarding accepts a legacy scalar field inside version 1 and rejects version 2. | Test a codec that upgrades and requests a new save, plus an unsupported version that performs no write or delete. No product proves a version-to-version migration. |
| Submit success | Redemut calls `clearAfterSave` after dialog, pack, and rehearsal submissions, and tests clearing pending and persisted data. Onboarding clears after its mutation succeeds. | Keep server success separate from local delete success. The form value may remain visible while the stored draft is cleared. |
| Failed draft deletion | The generic Redemut controller propagates delete failure. Onboarding swallows it, which can restore a completed draft on the next visit. There is no test. | Report `DraftPersistenceError` with operation `delete`, keep state truthful, and let the product offer retry or suppress recovery after confirmed submission. |

The future implementation also needs compatibility tests for Redemut's version 1 envelope and newest-value coalescing, plus a generated fixture because each helper starts from one source product. No storage migration is required if Redemut initially adapts its current draft table to the shared store interface.

## Decision

Decision: implement two independent framework-free helpers as opt-in exports from `@baukit/data-contracts`, a revisioned write queue and a durable draft helper. Do not combine them. Leitbild's two clients prove that acknowledged-revision serialization must work across browser and React Native code. Redemut's SQLite and Dexie paths prove that draft lifecycle is separate from its storage engine. The smallest next step is to write package-level conformance tests for every case above, then implement the interfaces against injected ports and add a generated non-React fixture. Product adoption must replace Leitbild's two queue implementations and Redemut's `FormDraftController`; publishing helpers without those deletions does not complete the item.

## What stays product-owned

- React and React Native hooks, mount policy, page-hide and navigation events, and status rendering.
- Request and response fields, revision representation, API error mapping, retry policy for unknown outcomes, and conflict resolution.
- Form values, schemas, codecs, version migrations, initial values, validation, submission, and recovery copy.
- Identity and document key inputs. Baukit's existing scoped persistence lifecycle continues to derive and mount opaque identity partitions.
- Storage schema and adapter migration. Redemut may keep its current SQLite and Dexie draft tables behind a thin adapter during adoption.
- Debounce duration, analytics, retention, discard confirmation, and whether a failed cleanup should block navigation.
