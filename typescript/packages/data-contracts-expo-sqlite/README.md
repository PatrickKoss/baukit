# `@baukit/data-contracts-expo-sqlite`

A zero-product-logic Expo SQLite implementation of `@baukit/data-contracts`' `RecordStore` contract.

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

Namespaces share one fixed `baukit_records` table without colliding. Call `initialize()` before using a store. Records are serialized as JSON, pagination is bounded and keyset-based, and malformed persisted payloads produce a content-free error. The package does not choose a database name, open a singleton, define product entities, or implement product cache policy.
