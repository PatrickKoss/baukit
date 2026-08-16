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
transaction. Independent root calls are serialized. `close()` closes the
logical adapter; pass `{ closeDatabase: true }` when it should also own the
supplied database handle.

Namespaces share one fixed `baukit_records` table without colliding. Call `initialize()` before using a store. Records are serialized as JSON, pagination is bounded and keyset-based, and malformed persisted payloads produce a content-free error. The package does not choose a database name, open a singleton, define product entities, or implement product cache policy.
