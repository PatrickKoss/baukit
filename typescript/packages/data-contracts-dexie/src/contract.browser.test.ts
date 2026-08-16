import type { ContractTestRecord } from '@baukit/data-contracts/vitest';
import { describeTransactionalStorageContract } from '@baukit/data-contracts/vitest';
import { afterEach } from 'vitest';

import { type DexieStore, openDexieStore } from './index.js';

const stores: DexieStore<ContractTestRecord>[] = [];
let nextDatabase = 0;

async function makeStore(): Promise<DexieStore<ContractTestRecord>> {
  const name = `baukit-browser-contract-${String(++nextDatabase)}-${crypto.randomUUID()}`;
  const store = await openDexieStore<ContractTestRecord>(name);
  stores.push(store);
  return store;
}

afterEach(async () => {
  await Promise.all(stores.splice(0).map((store) => store.close()));
});

describeTransactionalStorageContract(makeStore);
