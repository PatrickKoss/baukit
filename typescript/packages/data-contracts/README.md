# `@baukit/data-contracts`

Provider-neutral, asynchronous storage contracts plus executable adapter conformance tests.

The package defines JSON key/value storage, ID-ordered record storage with bounded keyset pagination, atomic transaction callbacks, and schema metadata/migration conventions. It deliberately contains no product entities and no Expo SQLite, Dexie, or Node database adapters.

## Using the contracts

```ts
import type { StorageTransaction, StoredRecord, Transaction } from '@baukit/data-contracts';

interface Note extends StoredRecord {
  title: string;
}

type NoteDatabase = StorageTransaction<Note> & Transaction<StorageTransaction<Note>>;
```

`RecordStore.list` orders immutable string IDs ascending using JavaScript string comparison. Page sizes are limited to `1..MAX_PAGE_SIZE`, and continuation cursors must be opaque to callers. Adapters should implement keyset cursors based on the last returned ID, not numeric offsets, so insertion before a cursor cannot shift later pages.

## Proving an adapter

Vitest helpers are isolated in a test-only subpath. Vitest is deliberately not a peer dependency, so it is never installed in Jest or production consumers. Install Vitest in an adapter project's development dependencies before importing the subpath, then register the applicable suites:

```ts
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeTransactionContract,
} from '@baukit/data-contracts/vitest';

const makeDatabase = () => new MyAdapter();

describeKeyValueContract(() => makeDatabase().keyValues);
describeRecordStoreContract(() => makeDatabase().records);
describeTransactionContract(makeDatabase);
describeSchemaMetadataContract(() => makeDatabase().schemaMetadata);
```

Each factory must return a fresh, empty store. The record suite supplies its own JSON-shaped `{ id, label, payload }` records. A transaction implementation must expose a callback-scoped view and make all callback writes visible together, or none if the callback throws/rejects. The included `InMemoryStore` exposes `keyValues`, `records`, and `schemaMetadata` namespaces and is itself tested by every suite.

Persistence adapters and product-specific migration logic belong in product or future adapter packages. The production entry point has no runtime dependencies; only the `/vitest` subpath expects the consumer's Vitest installation.
