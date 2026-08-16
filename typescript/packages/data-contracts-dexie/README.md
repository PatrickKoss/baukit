# `@baukit/data-contracts-dexie`

A product-neutral Dexie 4.x implementation of `@baukit/data-contracts` for web
applications that need key/value data, ID-ordered records, portable schema
metadata, and compound write transactions.

```ts
import { openDexieStore } from '@baukit/data-contracts-dexie';

interface CachedRecord {
  readonly id: string;
  readonly value: string;
}

const storage = await openDexieStore<CachedRecord>('product-cache');
await storage.withTransaction(async (transaction) => {
  await transaction.records.put({ id: 'record-1', value: 'local' });
  await transaction.keyValues.set('outbox:mutation-1', {
    entityId: 'record-1',
    operation: 'put',
  });
});
await storage.close();
```

The package owns three generic IndexedDB object stores. It does not define
product tables, entity relationships, soft-delete behavior, revisions, sync
protocols, or database naming policy.

Authenticated products should resolve an opaque name with
`@baukit/data-contracts` before calling `openDexieStore`. The fake-IndexedDB and
Chromium/WebKit suites run the shared offline E→F→E identity-transition cases
in addition to the transaction contract.

## Transaction model

Nested transactions explicitly join the ambient transaction when they are
started through the callback's transaction-scoped context:

```ts
await storage.withTransaction((transaction) =>
  transaction.withTransaction((sameTransaction) => sameTransaction.records.put(record)),
);
```

Compose transactional repositories from that scoped context. A call on the
root `storage` object is an independent transaction and is serialized behind
transactions that already started. Transaction contexts must not be retained
after their callback ends.

Prepare network, native, crypto, filesystem, or other external promise results
before opening a write transaction. Dexie tracks its own promise zone, but an
unrelated promise layer can let the browser's IndexedDB request queue drain and
auto-commit early. Keep the callback limited to the complete local write set.

Do not blanket-ignore Dexie's `PrematureCommitError`. That error can mean the
browser committed only an early subset of the intended writes. Treating it as
success without proof can silently split a compound operation. Acceptance
tests must observe every intended write, the matching outbox entry, and full
rollback when any step fails.

Quota failures surface as `StorageError` with code
`storage_quota_exceeded`. After `close()` resolves, operations fail with code
`storage_closed`.

## Verification

`pnpm test` uses `fake-indexeddb` and stays fast. The browser suite is a
separate Chromium and WebKit gate. Install browsers into a repository-local
cache and run it from the repository root:

```bash
PLAYWRIGHT_BROWSERS_PATH="$PWD/typescript/.playwright-browsers" \
  corepack pnpm --dir typescript --filter @baukit/data-contracts-dexie \
  exec playwright install --with-deps chromium webkit
PLAYWRIGHT_BROWSERS_PATH="$PWD/typescript/.playwright-browsers" \
  corepack pnpm --dir typescript --filter @baukit/data-contracts-dexie \
  run test:browser
```

Run `corepack pnpm --dir typescript run check` for the normal workspace gate.
The package-local `check` script also includes the browser suite and therefore
expects the two browser binaries to be installed. CI should add the same
repository-local browser install and `test:browser` commands as a distinct
step; the workspace's ordinary `test` task intentionally remains fast.
