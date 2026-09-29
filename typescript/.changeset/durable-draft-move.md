---
'@baukit/data-contracts': patch
---

Add `DurableDraft.move(scope)`, which hands an open draft to another scope, for example from a "new" draft to the record the server created. It writes a dirty or stored value under the new key, deletes the old key, keeps the value, revision, and submission state, and sends later saves to the new key. It resolves `moved`, `blocked`, or `stale` and rejects with `DraftPersistenceError` on a storage failure, leaving the draft on the old scope.

Add the `isScopeActive` option to `createDurableDraft`. The helper calls it before every write or delete; when it returns false, `save`, `clear`, and `move` resolve `stale` without touching storage. Reads are not checked.

Breaking: the `DurableDraft` interface gains the required `move` method, so a product's own implementation of that interface must add it. Code that only calls `createDurableDraft` is unaffected.
