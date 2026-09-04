# `@baukit/data-contracts`

Runtime-neutral data contracts, measurement helpers, and executable adapter conformance tests.

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

## Resource-budget measurements

Import production measurements from the `/limits` subpath. Checks return the measured and allowed
values, or throw `LimitExceededError` with those same fields. Products choose each allowed value and
map the error into their own reason code.

```ts
import {
  checkCompactJsonUtf8Bytes,
  checkTrimmedUnicodeScalars,
} from '@baukit/data-contracts/limits';

checkTrimmedUnicodeScalars('  e\u0301  ', 2);
checkCompactJsonUtf8Bytes({ value: 'é' }, 14);
```

Text measurement trims Unicode `White_Space` scalars at both ends. It does not normalize text.
Compact JSON measurement accepts null, booleans, finite numbers, scalar-only strings, dense arrays,
and plain objects. Plain objects may use enumerable own string keys. Non-enumerable properties are
ignored. Symbol keys, accessors, custom prototypes, circular references, unsupported values,
non-finite numbers, and unpaired surrogates throw `ResourceMeasurementError`. The compact encoder
uses `JSON.stringify` property order, though property order cannot change the measured byte count.

Existing product helpers can migrate one call at a time. Replace `codePointLength(value.trim())`
with `trimmedUnicodeScalarCount(value)`, and replace a `JSON.stringify` plus `TextEncoder` byte count
with `compactJsonUtf8Bytes(value)`. Unlike raw `JSON.stringify`, the helper rejects values that JSON
would omit or replace with `null`.

## Authenticated partitions

`deriveScopedStoreName(namespace, subject)` hashes a length-delimited canonical
identity with SHA-256 and returns an opaque name. Browser and Node runtimes use
`globalThis.crypto.subtle`. React Native must install a standards-compatible
Web Crypto polyfill before calling the default helper, or inject a
`ScopedPersistenceDigest`; the generated Expo template injects `expo-crypto`.

Keep `ScopedPersistenceRegistryStore` outside the domain database (for example,
SecureStore or localStorage). `resolveScopedStore` serializes access through one
registry instance, validates all versioned metadata, and only claims a legacy
store when an explicit inspector returns `claimable` or `current-subject`.
Malformed, unknown-version, inconsistent, or digest-mismatched metadata throws
`PersistenceIdentityMismatchError` with code
`persistence_identity_mismatch` before a domain store is opened.
Keep the configured legacy store name available after a successful claim: every
reopen verifies the recorded name against that configuration and fails closed
if it is missing or changed. A store name may belong to only one registry entry,
including across namespaces.

`ScopedPersistenceLifecycle` immediately hides stale handles, closes before it
opens another subject, resets product-provided user-scoped memory, and publishes
only an open/migrated/hydrated partition. Call `handleSessionExpired()` for a
terminal authentication expiry; it closes and blocks without inventing a
subject switch. Products with an older, already-versioned ownership registry
may supply `resolveStore` to retain those database names while adopting the
shared close/reset/publish lifecycle. The compatibility resolver remains
responsible for validating its legacy metadata and failing closed. Use
`recheckServerSubjectBeforeSyncAdoption` immediately before server identity
adoption or an outbox push.

`RecordStore.list` orders immutable string IDs ascending using JavaScript string comparison. Page sizes are limited to `1..MAX_PAGE_SIZE`, and continuation cursors must be opaque to callers. Adapters should implement keyset cursors based on the last returned ID, not numeric offsets, so insertion before a cursor cannot shift later pages.

## Proving an adapter

Vitest helpers are isolated in a test-only subpath. Vitest is deliberately not a peer dependency, so it is never installed in Jest or production consumers. Install Vitest in an adapter project's development dependencies before importing the subpath, then register the applicable suites:

```ts
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeScopedPersistenceContract,
  describeTransactionContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';

const makeDatabase = () => new MyAdapter();

describeKeyValueContract(() => makeDatabase().keyValues);
describeRecordStoreContract(() => makeDatabase().records);
describeTransactionContract(makeDatabase);
describeTransactionalStorageContract(makeDatabase);
describeScopedPersistenceContract(makeNamedDatabaseAdapter);
describeSchemaMetadataContract(() => makeDatabase().schemaMetadata);
```

Each factory must return a fresh, empty store. The record suite supplies its
own JSON-shaped `{ id, label, payload }` records. A transaction implementation
must expose a callback-scoped view and make all callback writes visible
together, or none if the callback throws/rejects. The stronger composite suite
also proves callback results, compound and write-plus-outbox-shaped atomicity,
joined reentrancy, quota normalization, close behavior, and concurrent
transaction serialization. The scoped suite proves offline E→F→E record and
outbox isolation, close-before-open ordering, memory reset, legacy claims,
corrupt-registry blocking, server-subject checks, and terminal expiry. Its
adapter factory must reopen the same durable data for the same name. The
included `InMemoryStore` exposes `keyValues`,
`records`, and `schemaMetadata` namespaces and is itself tested by every suite.

Persistence adapters and product-specific migration logic belong in product or future adapter packages. The production entry point has no runtime dependencies; only the `/vitest` subpath expects the consumer's Vitest installation.
