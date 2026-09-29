---
'@baukit/data-contracts-expo-sqlite': patch
---

`SqliteTransaction.transaction(work)` now nests instead of rejecting. It runs `work` in a `SAVEPOINT` on the same handle without waiting on the per-file queue, releases it when `work` resolves, and rolls back to it when `work` rejects, so only the nested statements and schema changes are undone and the same error reaches the caller. The enclosing transaction rolls back too unless the caller catches that error. While a nested transaction is open, statements and `transaction()` on an enclosing context reject with a `TypeError`.

Break: `SqliteTransaction.transaction` changes from `(work) => Promise<never>` to `<TResult>(work: (transaction: SqliteTransaction) => Promise<TResult> | TResult) => Promise<TResult>`. Code that relied on a nested call rejecting, or that typed its forwarding method as returning `Promise<never>`, must change.
