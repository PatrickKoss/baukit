# `@baukit/data-contracts-expo-sqlite`

A zero-product-logic Expo SQLite implementation of `@baukit/data-contracts`'
base storage contracts.

```ts
import { SqliteRecordStore } from '@baukit/data-contracts-expo-sqlite';
import * as SQLite from 'expo-sqlite';

interface CachedRecord {
  readonly id: string;
  readonly value: string;
}

const database = await SQLite.openDatabaseAsync('product.db');
const records = new SqliteRecordStore<CachedRecord>(database, 'cached-records');
await records.initialize();
```

For key/value data, schema metadata, and atomic compound writes, use the
composite adapter:

```ts
import { ExpoSqliteStore } from '@baukit/data-contracts-expo-sqlite';

const storage = new ExpoSqliteStore<CachedRecord>(database, 'product');
await storage.initialize();
await storage.withTransaction((transaction) =>
  transaction.withTransaction((sameTransaction) =>
    sameTransaction.records.put({ id: 'one', value: 'cached' }),
  ),
);
```

Nested calls made on the transaction-scoped context join the ambient exclusive
transaction. `close()` closes the logical adapter; pass
`{ closeDatabase: true }` when it should also own the supplied database handle.

Namespaces share one fixed `baukit_records` table without colliding. Call `initialize()` before using a store. Records are serialized as JSON, pagination is bounded and keyset-based, and malformed persisted payloads produce a content-free error.

## Operation ordering

Expo SQLite runs `withExclusiveTransactionAsync` on a second connection to the
same file. A statement on the base handle that runs while that transaction is
open fails with `database is locked`. The adapter therefore runs root reads,
root writes, `initialize()`, `withTransaction()`, and `close()` one at a time,
in call order, on one queue per database file (`SQLiteDatabase.databasePath`).
Every `ExpoSqliteStore`, `SqliteRecordStore`, `SqliteKeyValueStore`, and
`SqliteSchemaMetadataStore` on that file shares the queue, whichever handle and
namespace it uses. Stores on different files do not wait for each other.

- Work on the transaction context runs on the transaction connection and never
  enters the queue.
- A rejected operation releases the queue for the next caller.
- `close()` rejects new calls with `storage_closed`, waits for work it already
  accepted, closes an owned handle once, and returns the same promise to every
  caller, including a rejected one.
- Lock errors from SQLite reach the caller unchanged. The adapter never retries.

Inside a `withTransaction` callback, use the context it passes in. A root call
on any store for the same file queues behind the transaction, the transaction
waits for that call, and neither finishes. Keep network and file work outside the
callback, because it holds the queue for every store on that file. Raw
statements a product runs on the handle itself bypass the queue.

The package's fast Vitest suite runs the shared contracts against a deterministic
database fake and against `NodeSqliteDatabase` (see below). The
[Expo SQLite device-conformance app](../../../examples/expo-sqlite-conformance/README.md)
mirrors the shared contract cases against real `expo-sqlite` on Android,
including creation/reopening, namespace isolation, malformed data, rollback,
schema-metadata upgrades, root-versus-transaction overlap in both arrival
orders, and authenticated E→F→E database isolation. Products
derive the database name and resolve its registry with `@baukit/data-contracts`
before opening the Expo database. iOS is a scheduled/manual macOS gate.

## Node tests

`@baukit/data-contracts-expo-sqlite/testing` runs Expo SQLite code in Node unit
tests (Vitest or Jest) on Node's built-in `node:sqlite`. It needs Node 24 or
later and adds no dependency.

```ts
import { ExpoSqliteStore } from '@baukit/data-contracts-expo-sqlite';
import { NodeSqliteDatabase } from '@baukit/data-contracts-expo-sqlite/testing';

const database = new NodeSqliteDatabase();
const storage = new ExpoSqliteStore<CachedRecord>(database, 'product', { closeDatabase: true });
await storage.initialize();
```

`NodeSqliteDatabase` implements `ExpoSqliteDatabase`, the part of an Expo
`SQLiteDatabase` that this adapter calls: `databasePath`, `execAsync`,
`runAsync`, `getFirstAsync`, `getAllAsync`, `withExclusiveTransactionAsync`, and
`closeAsync`. A product driver typed against those methods accepts it without a
cast. With no argument it opens a new temporary file and deletes it on
`closeAsync()`. With a path it opens that file and leaves it in place, so a test
can close and reopen the same database. `:memory:` throws a `TypeError`, because
an exclusive transaction must open a second connection to the same database.

It copies the device behavior that product tests tend to hide:

- `withExclusiveTransactionAsync` runs `BEGIN`, the task, and `COMMIT` on a new
  connection to the same file, and rolls back when the task throws. A write on
  the root handle while that transaction is open fails with `database is locked`.
- Every connection opens with foreign keys off. `PRAGMA foreign_keys = ON` on
  the root handle does not reach transaction connections.
- Booleans and integral numbers bind as integers, `undefined` binds as NULL, and
  named parameters need their `$`, `:`, or `@` prefix.
- Rows are plain objects, and `getFirstAsync` returns `null` for no row.

It differs from the device in three places. An unknown named parameter or an
extra positional value throws instead of being ignored. A root read during an
open transaction succeeds and sees the committed state. An integer outside the
safe range throws. It has no `withTransactionAsync`, `prepareAsync`, or
synchronous methods.

Jest projects that transform `@baukit/*` through Babel resolve the entry through
its `default` condition.

## Migration runner conformance

`createSqliteMigrationConformanceTests(adapter)` returns framework-neutral cases
for a product's own SQLite migration runner. Register each with your test
runner:

```ts
import {
  createSqliteMigrationConformanceTests,
  type SqliteMigrationConformanceAdapter,
} from '@baukit/data-contracts-expo-sqlite/testing';

const adapter: SqliteMigrationConformanceAdapter = {
  migrate: (database, steps) => runMigrations(new ProductSqliteDriver(database), steps),
};

for (const testCase of createSqliteMigrationConformanceTests(adapter)) {
  it(testCase.name, testCase.run);
}
```

`migrate` receives a `NodeSqliteDatabase` and a list of
`{ version, name, sql }` steps, the complete list that one app build ships. It
must reject when the runner fails or refuses the database. The cases check that
the runner:

- applies every step to a fresh database;
- upgrades an older database and keeps its rows;
- applies nothing again after a restart;
- rolls back a failed step completely, with no table or column from its earlier
  statements left behind, and applies the fixed step after a restart;
- refuses a database that a newer build migrated, and leaves it unchanged.

The suite does not read the runner's history table or `user_version`, so any
runner that records versions somewhere passes. A step's `sql` holds several
statements, so the runner must execute it as a script (`execAsync`). Each case
uses its own temporary file. A failed case throws an error that starts with
`SQLite migration conformance failed:` and names the broken expectation.

## Boundaries

The package implements `@baukit/data-contracts` on Expo SQLite and nothing else. It does not choose a
database name, open a singleton, define product entities, or implement cache policy.

Passing the database handle in rather than opening one is what makes an authenticated product able to
derive its database name per identity, which is what the shared E→F→E isolation cases exercise.

`@baukit/data-contracts-dexie` is the same contract on the web.
