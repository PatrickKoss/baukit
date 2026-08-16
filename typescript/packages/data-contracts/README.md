# `@baukit/data-contracts`

Provider-neutral, asynchronous storage contracts plus executable adapter conformance tests.

The package defines JSON key/value storage, ID-ordered record storage with bounded keyset pagination, atomic transaction callbacks, and schema metadata/migration conventions. It deliberately contains no product entities and no Expo SQLite, Dexie, or Node database adapters.

## Using the contracts

```ts
import type { StoredRecord, TransactionalStorageStore } from '@baukit/data-contracts';

interface Note extends StoredRecord {
  title: string;
}

type NoteDatabase = TransactionalStorageStore<Note>;
```

`TransactionalStorageStore` makes nesting and lifecycle behavior explicit.
Inside a callback, call `withTransaction` on the transaction-scoped context to
join the ambient transaction. Calls on the root store are independent and must
be serialized by the adapter. After `close()` resolves, all operations reject
with `StorageError.code === "storage_closed"`. Quota failures use
`storage_quota_exceeded`; callers never need to parse provider error text.

The older `StorageTransaction` and `Transaction` interfaces remain available
for adapters that implement only the original surface.

`RecordStore.list` orders immutable string IDs ascending using JavaScript string comparison. Page sizes are limited to `1..MAX_PAGE_SIZE`, and continuation cursors must be opaque to callers. Adapters should implement keyset cursors based on the last returned ID, not numeric offsets, so insertion before a cursor cannot shift later pages.

## Proving an adapter

Vitest helpers are isolated in a test-only subpath. Vitest is deliberately not a peer dependency, so it is never installed in Jest or production consumers. Install Vitest in an adapter project's development dependencies before importing the subpath, then register the applicable suites:

```ts
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeTransactionContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';

const makeDatabase = () => new MyAdapter();

describeKeyValueContract(() => makeDatabase().keyValues);
describeRecordStoreContract(() => makeDatabase().records);
describeTransactionContract(makeDatabase);
describeTransactionalStorageContract(makeDatabase);
describeSchemaMetadataContract(() => makeDatabase().schemaMetadata);
```

Each factory must return a fresh, empty store. The record suite supplies its
own JSON-shaped `{ id, label, payload }` records. A transaction implementation
must expose a callback-scoped view and make all callback writes visible
together, or none if the callback throws/rejects. The stronger composite suite
also proves callback results, compound and write-plus-outbox-shaped atomicity,
joined reentrancy, quota normalization, close behavior, and concurrent
transaction serialization. The included `InMemoryStore` exposes `keyValues`,
`records`, and `schemaMetadata` namespaces and is itself tested by every suite.

Persistence adapters and product-specific migration logic belong in product or future adapter packages. The production entry point has no runtime dependencies; only the `/vitest` subpath expects the consumer's Vitest installation.
