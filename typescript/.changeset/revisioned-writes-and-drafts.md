---
'@baukit/data-contracts': minor
---

Add two opt-in entry points, neither re-exported from the package root and neither depending on React.

`/revisioned-writes` adds `createRevisionedWriteQueue`. It sends one write at a time with the last acknowledged revision, keeps edits made during a write as a separate unsent value, and combines queued edits with a caller `coalesce` function. The write callback returns `accepted`, `conflict`, or `rejected`. A conflict pauses the queue until `reset`. A throw means the outcome is unknown: the queue keeps the exact value and revision apart from newer edits, and `retry` sends them again with `afterUnknownOutcome: true`. `reset` fences account and document switches and aborts the in-flight write through the injected `AbortSignal`. The snapshot works with `useSyncExternalStore`.

`/durable-draft` adds `createDurableDraft`, which keeps a form value in any `KeyValueStore` as `{ version, value }` through a versioned codec that validates unchecked JSON. The snapshot reports `recovery` (`none`, `restored`, `corrupt`, `unsupported-version`, `unavailable`), `persistence` (`loading`, `idle`, `saving`, `clearing`, `failed`), `dirty`, and `localRevision`. `clear` takes `submitted` with the confirmed local revision, which keeps newer edits, or `discarded`. A failed deletion stays in the snapshot. Storage failures throw `DraftPersistenceError` without keys or draft content.

No breaking changes.
