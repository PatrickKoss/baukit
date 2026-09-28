# @baukit/data-contracts-expo-sqlite

## 0.5.0

### Minor Changes

- f57396c: Add the `@baukit/data-contracts-expo-sqlite/testing` entry point for Node unit tests on the built-in `node:sqlite` (Node 24 or later, no new dependency). `NodeSqliteDatabase` implements the Expo statement methods this adapter calls, on a file path it owns or one you pass in. Exclusive transactions open a second connection to the same file, as on the device, so an overlapping root write fails with `database is locked`, and foreign keys stay off on every connection. `createSqliteMigrationConformanceTests(adapter)` returns framework-neutral cases that check a product migration runner for fresh installs, upgrades, restarts, complete rollback of a failed step, and refusal of a database migrated by a newer build.

  Export `ExpoSqliteDatabase`, the part of an Expo `SQLiteDatabase` the adapter calls. The `ExpoSqliteStore` constructor now accepts that type instead of `SQLiteDatabase`, so an Expo database still fits.

  No breaking changes.

- c795f17: Serialize Expo SQLite operations per database file. Root record, key/value, and schema-metadata calls, `initialize()`, `withTransaction()`, and `close()` now run one at a time in call order on one queue shared by every adapter store on the same `databasePath`, so a root statement can no longer overlap an exclusive transaction and fail with `database is locked`. Transaction-context work stays on the transaction connection. Lock errors still reach the caller unchanged; nothing is retried.

  Breaking changes:

  - `SqliteRecordStore`, `SqliteKeyValueStore`, and `SqliteSchemaMetadataStore` constructors take `(database, namespace)` only. The third `assertAvailable` argument is removed, and `database` must expose `databasePath` (an Expo `SQLiteDatabase` does).
  - `ExpoSqliteStore.records`, `.keyValues`, and `.schemaMetadata` are typed as the `RecordStore`, `KeyValueStore`, and `SchemaMetadataStore` contracts. Their per-store `initialize()` is no longer part of the public type; call `ExpoSqliteStore.initialize()`.
  - A transaction accepted before `close()` now runs before the adapter closes instead of failing with `storage_closed`.
  - `close()` returns the same promise to every caller. A failed close keeps rejecting with the same error instead of resolving on the next call.
  - A root call made from inside a `withTransaction` callback on a store for the same file now waits forever instead of failing with `database is locked`. Use the transaction context inside the callback.
  - A product wrapper that queued root operations and transactions around the adapter becomes redundant; remove it after pinning this release.

- d4aca9d: Add `ExpoSqliteConnection`, a raw SQL connection for a product's own schema and migration runner. `exec`, `run`, `get`, and `all` bind parameters as `expo-sqlite` does. Root statements, `transaction(work)`, and `close()` share the per-file queue with `ExpoSqliteStore` and the root stores, so a root statement issued during another caller's transaction runs after it instead of joining it. `transaction(work)` runs `BEGIN IMMEDIATE` on the supplied handle, commits when `work` resolves, and rolls back when `work` or the commit rejects. The transaction context's `transaction()` always rejects with a `TypeError`, and its statements reject with `storage_closed` once it settles. The connection runs `PRAGMA foreign_keys = ON` before its first statement, so foreign keys stay enforced inside transactions. `close()` closes the handle only with `{ closeDatabase: true }`. New types: `ExpoSqliteConnectionOptions`, `SqliteStatements`, and `SqliteTransaction`.

  `ExpoSqliteStore` now shares its queueing, close, and transaction-context code with the connection. Its behavior is unchanged.

  Behavior change for code that shares a handle: the first statement of an `ExpoSqliteConnection` turns foreign keys on for that handle, so `ExpoSqliteStore` and root stores on the same handle run with foreign keys on from then on. The adapter's own tables have no foreign keys.

  No breaking changes.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

### Patch Changes

- Updated dependencies [8d268e1]
- Updated dependencies [acaab1b]
- Updated dependencies
- Updated dependencies [d9225d5]
- Updated dependencies [a9fa22e]
  - @baukit/data-contracts@0.5.0

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.4.0

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

### Patch Changes

- Updated dependencies [40882f6]
- Updated dependencies
- Updated dependencies [5472d3d]
  - @baukit/data-contracts@0.3.0

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.2.1

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.2.0

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.
- Updated dependencies
  - @baukit/data-contracts@0.1.2

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.1.1

## 0.1.0

### Minor Changes

- First public release of `@baukit/data-contracts-expo-sqlite`.
