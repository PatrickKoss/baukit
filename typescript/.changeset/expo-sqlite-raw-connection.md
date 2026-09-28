---
'@baukit/data-contracts-expo-sqlite': minor
---

Add `ExpoSqliteConnection`, a raw SQL connection for a product's own schema and migration runner. `exec`, `run`, `get`, and `all` bind parameters as `expo-sqlite` does. Root statements, `transaction(work)`, and `close()` share the per-file queue with `ExpoSqliteStore` and the root stores, so a root statement issued during another caller's transaction runs after it instead of joining it. `transaction(work)` runs `BEGIN IMMEDIATE` on the supplied handle, commits when `work` resolves, and rolls back when `work` or the commit rejects. The transaction context's `transaction()` always rejects with a `TypeError`, and its statements reject with `storage_closed` once it settles. The connection runs `PRAGMA foreign_keys = ON` before its first statement, so foreign keys stay enforced inside transactions. `close()` closes the handle only with `{ closeDatabase: true }`. New types: `ExpoSqliteConnectionOptions`, `SqliteStatements`, and `SqliteTransaction`.

`ExpoSqliteStore` now shares its queueing, close, and transaction-context code with the connection. Its behavior is unchanged.

Behavior change for code that shares a handle: the first statement of an `ExpoSqliteConnection` turns foreign keys on for that handle, so `ExpoSqliteStore` and root stores on the same handle run with foreign keys on from then on. The adapter's own tables have no foreign keys.

No breaking changes.
