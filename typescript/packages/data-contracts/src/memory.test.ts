import { describe, expect, it } from 'vitest';

import type { JsonValue, StoredRecord } from './contracts.js';
import { InMemoryStore } from './memory.js';
import {
  describeKeyValueContract,
  describeRecordStoreContract,
  describeSchemaMetadataContract,
  describeTransactionContract,
} from './vitest.js';

interface TestRecord extends StoredRecord {
  readonly label: string;
  readonly payload: JsonValue;
}

const makeStore = (): InMemoryStore<TestRecord> => new InMemoryStore<TestRecord>();

describeKeyValueContract(() => makeStore().keyValues);
describeRecordStoreContract(() => makeStore().records);
describeTransactionContract(makeStore);
describeSchemaMetadataContract(() => makeStore().schemaMetadata);

describe('InMemoryStore schema migration hook', () => {
  it('advances matching schema metadata', async () => {
    const store = makeStore();
    await store.schemaMetadata.setSchemaMeta({ name: 'notes', version: 1 });
    await store.migrate({ name: 'notes', version: 1 }, { name: 'notes', version: 2 });
    expect(await store.schemaMetadata.getSchemaMeta()).toEqual({ name: 'notes', version: 2 });
  });

  it('rejects a mismatched source or non-forward target', async () => {
    const store = makeStore();
    await store.schemaMetadata.setSchemaMeta({ name: 'notes', version: 1 });
    await expect(
      store.migrate({ name: 'notes', version: 0 }, { name: 'notes', version: 2 }),
    ).rejects.toThrow('does not match');
    await expect(
      store.migrate({ name: 'notes', version: 1 }, { name: 'notes', version: 1 }),
    ).rejects.toThrow('greater');
  });
});
