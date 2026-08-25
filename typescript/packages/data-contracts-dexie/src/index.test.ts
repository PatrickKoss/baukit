import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeScopedPersistenceContract,
  describeTransactionalStorageContract,
} from '@baukit/data-contracts/vitest';
import { IDBFactory, IDBKeyRange } from 'fake-indexeddb';
import { afterEach, describe, expect, it } from 'vitest';

import { type DexieStore, openDexieStore } from './index.js';

const stores: DexieStore<ContractTestRecord>[] = [];
let nextDatabase = 0;

async function makeStore(): Promise<DexieStore<ContractTestRecord>> {
  const name = `baukit-dexie-contract-${String(++nextDatabase)}`;
  const store = await openDexieStore<ContractTestRecord>(name, {
    indexedDB: new IDBFactory(),
    IDBKeyRange,
  });
  stores.push(store);
  return store;
}

afterEach(async () => {
  await Promise.all(stores.splice(0).map((store) => store.close()));
});

describeKeyValueContract(async () => (await makeStore()).keyValues);
describeRecordStoreContract(async () => (await makeStore()).records);
describeSchemaMetadataContract(async () => (await makeStore()).schemaMetadata);
describeTransactionalStorageContract(makeStore);
describeScopedPersistenceContract(() => {
  const indexedDB = new IDBFactory();
  return {
    open: async (storeName) => {
      const store = await openDexieStore<ContractTestRecord>(storeName, {
        indexedDB,
        IDBKeyRange,
      });
      stores.push(store);
      return store;
    },
  };
});

describe('DexieStore', () => {
  it('isolates separate databases', async () => {
    const first = await makeStore();
    const second = await makeStore();
    await first.keyValues.set('same', 'first');
    await second.keyValues.set('same', 'second');
    await expect(first.keyValues.get('same')).resolves.toBe('first');
    await expect(second.keyValues.get('same')).resolves.toBe('second');
  });

  it('commits the complete write set across callback microtasks', async () => {
    const store = await makeStore();
    await store.withTransaction(async (transaction) => {
      await transaction.records.put({ id: 'a', label: 'first', payload: 1 });
      await Promise.resolve('callback microtask');
      await transaction.records.put({ id: 'b', label: 'second', payload: 2 });
    });
    await expect(store.records.list()).resolves.toMatchObject({
      items: [
        { id: 'a', label: 'first', payload: 1 },
        { id: 'b', label: 'second', payload: 2 },
      ],
    });
  });
});
