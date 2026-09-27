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

The package's fast Vitest suite uses a deterministic database fake. The
[Expo SQLite device-conformance app](../../../examples/expo-sqlite-conformance/README.md)
mirrors the shared contract cases against real `expo-sqlite` on Android,
including creation/reopening, namespace isolation, malformed data, rollback,
schema-metadata upgrades, root-versus-transaction overlap in both arrival
orders, and authenticated E→F→E database isolation. Products
derive the database name and resolve its registry with `@baukit/data-contracts`
before opening the Expo database. iOS is a scheduled/manual macOS gate.

## Boundaries

The package implements `@baukit/data-contracts` on Expo SQLite and nothing else. It does not choose a
database name, open a singleton, define product entities, or implement cache policy.

Passing the database handle in rather than opening one is what makes an authenticated product able to
derive its database name per identity, which is what the shared E→F→E isolation cases exercise.

`@baukit/data-contracts-dexie` is the same contract on the web.
