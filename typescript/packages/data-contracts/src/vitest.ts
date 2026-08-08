import { describe, expect, it } from 'vitest';

import {
  MAX_PAGE_SIZE,
  type JsonValue,
  type KeyValueStore,
  type RecordStore,
  type SchemaMetadataStore,
  type StorageTransaction,
  type StoredRecord,
  type Transaction,
} from './contracts.js';

export type ContractStoreFactory<TStore> = () => Promise<TStore> | TStore;

/** Canonical JSON-shaped record supplied to adapter contract suites. */
export interface ContractTestRecord extends StoredRecord {
  readonly label: string;
  readonly payload: JsonValue;
}

const RECORDS = {
  first: { id: 'b', label: 'second', payload: { position: 2 } },
  before: { id: 'a', label: 'inserted before cursor', payload: null },
  second: { id: 'c', label: 'third', payload: [3] },
  third: { id: 'd', label: 'fourth', payload: true },
} as const satisfies Record<string, ContractTestRecord>;

/** Registers the provider-neutral key/value contract in the current Vitest suite. */
export function describeKeyValueContract(makeStore: ContractStoreFactory<KeyValueStore>): void {
  describe('KeyValueStore contract', () => {
    it('round-trips every JSON value shape without leaking mutable references', async () => {
      const store = await makeStore();
      const value: JsonValue = {
        array: [null, true, 42, 'text'],
        nested: { ready: false },
      };
      await store.set('value', value);
      expect(await store.get('value')).toEqual(value);

      const loaded = (await store.get('value')) as { nested: { ready: boolean } };
      loaded.nested.ready = true;
      expect(await store.get('value')).toEqual(value);
    });

    it('returns undefined for missing keys and treats missing deletes as a no-op', async () => {
      const store = await makeStore();
      await expect(store.get('missing')).resolves.toBeUndefined();
      await expect(store.delete('missing')).resolves.toBeUndefined();
    });

    it('replaces, deletes, and clears values', async () => {
      const store = await makeStore();
      await store.set('first', 1);
      await store.set('first', 2);
      await store.set('second', 3);
      expect(await store.get('first')).toBe(2);
      await store.delete('first');
      expect(await store.get('first')).toBeUndefined();
      await store.clear();
      expect(await store.get('second')).toBeUndefined();
    });
  });
}

/** Registers CRUD, bounds, and stable keyset-pagination requirements. */
export function describeRecordStoreContract(
  makeStore: ContractStoreFactory<RecordStore<ContractTestRecord>>,
): void {
  describe('RecordStore contract', () => {
    it('puts, replaces, gets, and deletes records', async () => {
      const store = await makeStore();
      await store.put(RECORDS.first);
      expect(await store.get(RECORDS.first.id)).toEqual(RECORDS.first);
      await store.put({ ...RECORDS.first, label: 'replacement' });
      expect(await store.get(RECORDS.first.id)).toEqual({
        ...RECORDS.first,
        label: 'replacement',
      });
      await store.delete(RECORDS.first.id);
      expect(await store.get(RECORDS.first.id)).toBeUndefined();
      await expect(store.delete('missing')).resolves.toBeUndefined();
    });

    it('returns an empty terminal page', async () => {
      const store = await makeStore();
      await expect(store.list({ limit: 2 })).resolves.toEqual({
        items: [],
        nextCursor: null,
      });
    });

    it('returns a terminal page when count exactly equals page size', async () => {
      const store = await makeStore();
      await store.put(RECORDS.second);
      await store.put(RECORDS.first);
      await expect(store.list({ limit: 2 })).resolves.toEqual({
        items: [RECORDS.first, RECORDS.second],
        nextCursor: null,
      });
    });

    it('uses stable keyset cursors when a record is inserted before the cursor', async () => {
      const store = await makeStore();
      await store.put(RECORDS.first);
      await store.put(RECORDS.second);
      await store.put(RECORDS.third);

      const firstPage = await store.list({ limit: 1 });
      expect(firstPage.items).toEqual([RECORDS.first]);
      expect(firstPage.nextCursor).not.toBeNull();

      await store.put(RECORDS.before);
      const remaining = await store.list({ cursor: firstPage.nextCursor, limit: 2 });
      expect(remaining).toEqual({
        items: [RECORDS.second, RECORDS.third],
        nextCursor: null,
      });
    });

    it('rejects unbounded, fractional, and invalid cursor requests', async () => {
      const store = await makeStore();
      await expect(store.list({ limit: 0 })).rejects.toThrow();
      await expect(store.list({ limit: MAX_PAGE_SIZE + 1 })).rejects.toThrow();
      await expect(store.list({ limit: 1.5 })).rejects.toThrow();
      await expect(store.list({ cursor: 'not-an-adapter-cursor' })).rejects.toThrow();
    });
  });
}

export interface TransactionContractStore<T extends StoredRecord>
  extends StorageTransaction<T>, Transaction<StorageTransaction<T>> {}

/** Registers commit, result, and rollback-on-throw transaction requirements. */
export function describeTransactionContract(
  makeStore: ContractStoreFactory<TransactionContractStore<ContractTestRecord>>,
): void {
  describe('Transaction contract', () => {
    it('atomically commits and returns the callback result', async () => {
      const store = await makeStore();
      const result = await store.withTransaction(async (transaction) => {
        await transaction.keyValues.set('first', 1);
        await transaction.keyValues.set('second', 2);
        await transaction.records.put(RECORDS.first);
        await transaction.schemaMetadata.setSchemaMeta({ name: 'contract', version: 1 });
        return 'committed';
      });
      expect(result).toBe('committed');
      expect(await store.keyValues.get('first')).toBe(1);
      expect(await store.keyValues.get('second')).toBe(2);
      expect(await store.records.get(RECORDS.first.id)).toEqual(RECORDS.first);
      expect(await store.schemaMetadata.getSchemaMeta()).toEqual({
        name: 'contract',
        version: 1,
      });
    });

    it('rolls every write back when the callback throws', async () => {
      const store = await makeStore();
      await store.keyValues.set('preserved', 'before');
      await expect(
        store.withTransaction(async (transaction) => {
          await transaction.keyValues.set('preserved', 'after');
          await transaction.keyValues.set('new', true);
          await transaction.records.put(RECORDS.first);
          await transaction.schemaMetadata.setSchemaMeta({ name: 'contract', version: 1 });
          throw new Error('deliberate rollback');
        }),
      ).rejects.toThrow('deliberate rollback');
      expect(await store.keyValues.get('preserved')).toBe('before');
      expect(await store.keyValues.get('new')).toBeUndefined();
      expect(await store.records.get(RECORDS.first.id)).toBeUndefined();
      expect(await store.schemaMetadata.getSchemaMeta()).toBeUndefined();
    });
  });
}

/** Registers persistence and replacement behavior for schema metadata. */
export function describeSchemaMetadataContract(
  makeStore: ContractStoreFactory<SchemaMetadataStore>,
): void {
  describe('SchemaMetadataStore contract', () => {
    it('round-trips and replaces schema/version metadata', async () => {
      const store = await makeStore();
      await expect(store.getSchemaMeta()).resolves.toBeUndefined();
      await store.setSchemaMeta({ name: 'notes', version: 1 });
      await expect(store.getSchemaMeta()).resolves.toEqual({ name: 'notes', version: 1 });
      await store.setSchemaMeta({ name: 'notes', version: 2 });
      await expect(store.getSchemaMeta()).resolves.toEqual({ name: 'notes', version: 2 });
    });
  });
}
