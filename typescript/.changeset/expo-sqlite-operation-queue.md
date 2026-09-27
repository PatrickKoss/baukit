---
'@baukit/data-contracts-expo-sqlite': minor
---

Serialize Expo SQLite operations per database file. Root record, key/value, and schema-metadata calls, `initialize()`, `withTransaction()`, and `close()` now run one at a time in call order on one queue shared by every adapter store on the same `databasePath`, so a root statement can no longer overlap an exclusive transaction and fail with `database is locked`. Transaction-context work stays on the transaction connection. Lock errors still reach the caller unchanged; nothing is retried.

Breaking changes:

- `SqliteRecordStore`, `SqliteKeyValueStore`, and `SqliteSchemaMetadataStore` constructors take `(database, namespace)` only. The third `assertAvailable` argument is removed, and `database` must expose `databasePath` (an Expo `SQLiteDatabase` does).
- `ExpoSqliteStore.records`, `.keyValues`, and `.schemaMetadata` are typed as the `RecordStore`, `KeyValueStore`, and `SchemaMetadataStore` contracts. Their per-store `initialize()` is no longer part of the public type; call `ExpoSqliteStore.initialize()`.
- A transaction accepted before `close()` now runs before the adapter closes instead of failing with `storage_closed`.
- `close()` returns the same promise to every caller. A failed close keeps rejecting with the same error instead of resolving on the next call.
- A root call made from inside a `withTransaction` callback on a store for the same file now waits forever instead of failing with `database is locked`. Use the transaction context inside the callback.
- A product wrapper that queued root operations and transactions around the adapter becomes redundant; remove it after pinning this release.
