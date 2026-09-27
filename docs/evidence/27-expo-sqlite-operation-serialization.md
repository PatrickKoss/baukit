# Expo SQLite operation serialization evidence

Item 1 of the [cross-product feature plan](../cross-product-feature-plan.md).

## Source revisions

- Eigenruhe `f74cebb` (the survey read `e44ff88`; the files below did not change between them):
  `docs/tickets/baukit-sqlite-operation-serialization.md`, `mobile/src/db/serialized-store.ts`,
  `mobile/src/db/serialized-store.test.ts`, `mobile/src/local-data.ts:53-69`,
  `mobile/src/record-store.ts:21-66,96-112`.
- Leitbild `bd38b33`: `mobile/src/local-data.ts:5,52-63`.
- Redemut `a782538`: `mobile/src/record-store.ts:1-16`, `mobile/src/app-shell.tsx:65-68`.
- Hebkit `841bf5d`: `mobile/src/db/sqlite/adapters/expo-sqlite.ts:31-40` (private driver, not this
  adapter).
- Redemut `a782538`: `packages/data/src/expo-sqlite.ts:40-76` (private driver, not this adapter).
- `expo-sqlite` 57.0.1: `src/SQLiteDatabase.ts` (`withExclusiveTransactionAsync` and
  `Transaction.createAsync`), `ios/SQLiteModule.swift` and `android/.../SQLiteModule.kt`
  (native connection cache keyed by path).

## Observed failure

Eigenruhe's packaged Android app stopped session startup with `NativeStatement.finalizeAsync`
reporting `database is locked` on Expo SDK 57 and Android 16. The pre-session check-in commits a
transaction and requests sync; the opening bell reads cached sound metadata while sync reads and
writes its state.

`withExclusiveTransactionAsync` opens a new native connection to `databasePath` with
`useNewConnection: true`, runs `BEGIN`, the callback, and `COMMIT` on it, then closes it. Expo sets
no busy timeout, so a statement on any other connection to the same file fails at once while that
transaction holds its lock, and a transaction cannot commit while another connection reads. Baukit
0.4.0 queued only `withTransaction` calls on a per-instance `transactionTail`. Its root `records`,
`keyValues`, and `schemaMetadata` stores ran straight on the base handle, so they overlapped the
transaction connection in either arrival order.

## Baukit owner

`@baukit/data-contracts-expo-sqlite` (`typescript/packages/data-contracts-expo-sqlite`).

## Shared-connection decision

Coordinate by database identity, using the database file path (`SQLiteDatabase.databasePath`) as
the identity. Do not enforce one store per connection.

Reasoning:

- The lock belongs to the file, not to a JavaScript handle. Every exclusive transaction opens its
  own connection to the path, so a store on a second handle to the same file collides as surely as
  a store on the same handle. Expo also caches native connections by path, so two
  `openDatabaseAsync` calls for one name return two handles over one native connection. A queue
  keyed by handle object would miss both cases. A namespace-keyed queue would miss them and also
  two namespaces on one handle.
- Baukit already documents and tests several namespaced stores on one handle: the README says
  namespaces share one table without colliding, the package test "isolates records in separate
  namespaces sharing a database" builds two record stores on one handle, and the device-conformance
  app builds every contract store on one shared handle. Enforcing one store per connection would
  break those uses without making any product safer.
- Enforcement would still need a registry of claimed handles, which is the same module state a
  queue registry needs, and it could not see a second handle to the same file either.

The registry is a module-level `Map` from path to queue. An entry exists only while that file has
pending work and is removed when its queue goes idle, so closed or abandoned databases leave nothing
behind and independent files never share a queue. Two different in-memory databases share the
`:memory:` path and are over-serialized; that is harmless.

Known limits, documented in the package README:

- Raw statements a product runs on the handle itself bypass the queue.
- A root call on any adapter store for the same file, made from inside a `withTransaction`
  callback, queues behind the transaction that is waiting for it and never settles. Before this
  change it usually failed with `database is locked`. The adapter cannot tell such a call from a
  concurrent caller without async context, which React Native does not provide.

## Public types and errors

- `ExpoSqliteStore<T>`: constructor `(database: SQLiteDatabase, namespace, options?)`.
  `records`, `keyValues`, and `schemaMetadata` are typed as `RecordStore<T>`, `KeyValueStore`, and
  `SchemaMetadataStore`.
- `ExpoSqliteStoreOptions.closeDatabase` is unchanged.
- `SqliteRecordStore<T>`, `SqliteKeyValueStore`, `SqliteSchemaMetadataStore`: constructor
  `(database, namespace)`, where `database` exposes `databasePath`, `execAsync`, `getAllAsync`,
  `getFirstAsync`, and `runAsync`. Each shares the file queue.
- `StorageError` codes are unchanged: `storage_closed` for calls after `close()` or on a finished
  transaction context, `storage_quota_exceeded` for normalized quota failures.

No new exports. The queue (`src/operation-queue.ts`) and the statement classes
(`src/statements.ts`) are internal modules outside the package `exports` map.

## Contract and cases

Package tests (`src/index.test.ts`, `src/operation-queue.test.ts`) use a fake file that behaves like
SQLite without a busy timeout: a root statement fails with `database is locked` while a transaction
is open, and a transaction cannot begin while a root statement is in flight. Statements can be
paused and failed on demand.

- Root before transaction, for record write, read, and list, key/value write and read, and schema
  write and read: the transaction starts only after the paused root statement finishes.
- Transaction before root, for the same seven operations: the root call settles only after the
  transaction commits and observes its writes. `initialize()` also waits.
- Failure: a rejected root statement and a rolled-back transaction both release the queue.
- Lock errors: a `database is locked` failure reaches the caller as the same error object after one
  statement attempt.
- Nested context: three levels of nested `withTransaction` on the context complete, and a root call
  queued behind them runs afterwards.
- Close: accepted root work and an accepted transaction finish before the owned handle closes;
  calls made after `close()` reject with `storage_closed`; concurrent callers get one promise; the
  handle closes once; a failed close rejects every caller with the same error; an unowned handle
  stays open.
- Shared file: two namespaces on one handle, two handles on one path, and a standalone
  `SqliteRecordStore` next to an `ExpoSqliteStore` all wait for the transaction. A store on a
  different file proceeds while another file holds a transaction.
- Queue: call order, release after rejection, one idle signal, and registry cleanup when idle.

With the 0.4.0 adapter, 22 of these package cases fail. The device-conformance app adds three real
Expo SQLite cases on Android: twenty root writes plus key/value and schema writes before a
transaction that must observe all of them; root reads and writes for all three families during a
held transaction that must observe its commit; and a root write from a second namespace on the
same handle during another store's held transaction.

On an Android API 36 emulator with the 0.4.0 adapter, the first of these cases fails with
`Call to function 'NativeDatabase.prepareAsync' has been rejected. → Caused by: Error code :
database is locked`, the same native error Eigenruhe recorded. With this change all 26 device cases
pass. The runner now prefixes a failure with the case name.

`scripts/run-android.sh` now runs the example's `pnpm install` with `CI=1`, as CI already does. A
second local run in the same checkout used to stop with `ERR_PNPM_ABORTED_REMOVE_MODULES_DIR_NO_TTY`.
Host port 8081 was free for every run, so the `BAUKIT_METRO_PORT` override the plan mentions was not
needed. The script's existing `METRO_PORT` variable already moves Metro and points the app at it
through `adb reverse` and `debug_http_host`.

## Supported runtimes

Expo SDK 57 with `expo-sqlite` 57 on Android and iOS. Android runs in `make expo-sqlite-conformance`.
iOS belongs to the native release gate and was not run for this change. The web build of
`expo-sqlite` has no exclusive transactions; this adapter does not target the web.

## Failure behavior

A rejected operation rejects only its caller and releases the queue. SQLite errors, including
`database is locked`, pass through unchanged apart from the existing quota normalization. Nothing
retries, and a lock error is never reported as success. `close()` settles every caller with one
result.

## Privacy boundary

The queue holds closures and a file path in memory only. Nothing is logged. Malformed-row errors
stay content-free, as before.

## Breaks

- The third `assertAvailable` constructor argument of `SqliteRecordStore`, `SqliteKeyValueStore`,
  and `SqliteSchemaMetadataStore` is removed, and their `database` argument must expose
  `databasePath`.
- `ExpoSqliteStore.records`, `.keyValues`, and `.schemaMetadata` lose their per-store
  `initialize()` in the public type. No surveyed product calls it.
- A transaction accepted before `close()` now runs instead of failing with `storage_closed`.
- A failed `close()` keeps rejecting on later calls instead of resolving.
- A root call from inside a transaction callback on the same file now waits forever instead of
  failing with `database is locked`.

## Product adoption change

After Eigenruhe pins the release and passes its native regression journeys:

- Delete `mobile/src/db/serialized-store.ts` and `mobile/src/db/serialized-store.test.ts`.
- In `mobile/src/local-data.ts`, return the `ExpoSqliteStore` directly instead of wrapping it in
  `SerializedProductStore`.
- Delete or rewrite the wrapper case "finishes an accepted settings mutation before a serialized
  store closes" in `mobile/src/db/contract-tests/persistence.test.ts`; the adapter now owns that
  guarantee.
- Close `docs/tickets/baukit-sqlite-operation-serialization.md` and the QA-007 entry.
- Keep `SerializedRecordStoreResource` in `mobile/src/record-store.ts`. It orders preference calls
  with closing a `useNewConnection` handle that `SqliteRecordStore` does not own, which this change
  does not cover.

Leitbild (`mobile/src/local-data.ts`) and Redemut (`mobile/src/record-store.ts`) need only the
version bump; each opens one store per file. Hebkit's and Redemut's private drivers stay listed as
product defects; item 15 decides whether they move to this adapter.
