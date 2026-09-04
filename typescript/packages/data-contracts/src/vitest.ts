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
  type TransactionalStorageStore,
} from './contracts.js';
import {
  InMemoryScopedPersistenceRegistryStore,
  ScopedPersistenceLifecycle,
  recheckServerSubjectBeforeSyncAdoption,
} from './identity.js';

export * from './import-envelope.vitest.js';

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

function simulatedQuotaError(): Error {
  const error = new Error('simulated adapter quota');
  error.name = 'QuotaExceededError';
  return error;
}

/**
 * Registers lifecycle, reentrancy, compound atomicity, quota normalization,
 * and serialization requirements for a complete composite adapter.
 */
export function describeTransactionalStorageContract(
  makeStore: ContractStoreFactory<TransactionalStorageStore<ContractTestRecord>>,
): void {
  describe('TransactionalStorageStore contract', () => {
    it('propagates results and atomically commits a compound write', async () => {
      const store = await makeStore();
      const result = await store.withTransaction(async (transaction) => {
        await transaction.keyValues.set('first', 1);
        await transaction.keyValues.set('second', 2);
        await transaction.records.put(RECORDS.first);
        await transaction.records.put(RECORDS.second);
        await transaction.schemaMetadata.setSchemaMeta({ name: 'contract', version: 1 });
        return { status: 'committed' } as const;
      });
      expect(result).toEqual({ status: 'committed' });
      expect(await store.keyValues.get('first')).toBe(1);
      expect(await store.keyValues.get('second')).toBe(2);
      expect((await store.records.list()).items).toEqual([RECORDS.first, RECORDS.second]);
      expect(await store.schemaMetadata.getSchemaMeta()).toEqual({
        name: 'contract',
        version: 1,
      });
    });

    it('rolls every compound write back when the callback throws', async () => {
      const store = await makeStore();
      await store.keyValues.set('preserved', 'before');
      await expect(
        store.withTransaction(async (transaction) => {
          await transaction.keyValues.set('preserved', 'after');
          await transaction.records.put(RECORDS.first);
          await transaction.schemaMetadata.setSchemaMeta({ name: 'contract', version: 1 });
          throw new Error('deliberate rollback');
        }),
      ).rejects.toThrow('deliberate rollback');
      expect(await store.keyValues.get('preserved')).toBe('before');
      expect(await store.records.get(RECORDS.first.id)).toBeUndefined();
      expect(await store.schemaMetadata.getSchemaMeta()).toBeUndefined();
    });

    it('commits a record and outbox-shaped entry together', async () => {
      const store = await makeStore();
      await store.withTransaction(async (transaction) => {
        await transaction.records.put(RECORDS.first);
        await transaction.keyValues.set('outbox:mutation-1', {
          entityId: RECORDS.first.id,
          operation: 'put',
        });
      });
      expect(await store.records.get(RECORDS.first.id)).toEqual(RECORDS.first);
      expect(await store.keyValues.get('outbox:mutation-1')).toEqual({
        entityId: RECORDS.first.id,
        operation: 'put',
      });
    });

    it('joins nested calls made through the ambient transaction context', async () => {
      const store = await makeStore();
      const result = await store.withTransaction(async (transaction) => {
        await transaction.records.put(RECORDS.first);
        return transaction.withTransaction(async (nested) => {
          expect(nested).toBe(transaction);
          await nested.keyValues.set('nested', true);
          await nested.schemaMetadata.setSchemaMeta({ name: 'contract', version: 1 });
          return 'nested-result';
        });
      });
      expect(result).toBe('nested-result');
      expect(await store.records.get(RECORDS.first.id)).toEqual(RECORDS.first);
      expect(await store.keyValues.get('nested')).toBe(true);
    });

    it('rolls back joined nested writes when the outer callback fails', async () => {
      const store = await makeStore();
      await expect(
        store.withTransaction(async (transaction) => {
          await transaction.withTransaction(async (nested) => {
            await nested.records.put(RECORDS.first);
            await nested.keyValues.set('outbox:mutation-1', true);
          });
          throw new Error('outer failure');
        }),
      ).rejects.toThrow('outer failure');
      expect(await store.records.get(RECORDS.first.id)).toBeUndefined();
      expect(await store.keyValues.get('outbox:mutation-1')).toBeUndefined();
    });

    it('surfaces quota failures with a stable code and rolls writes back', async () => {
      const store = await makeStore();
      await expect(
        store.withTransaction(async (transaction) => {
          await transaction.records.put(RECORDS.first);
          throw simulatedQuotaError();
        }),
      ).rejects.toMatchObject({ code: 'storage_quota_exceeded' });
      expect(await store.records.get(RECORDS.first.id)).toBeUndefined();
    });

    it('fails operations after close with a stable code', async () => {
      const store = await makeStore();
      await store.close();
      await expect(store.keyValues.get('closed')).rejects.toMatchObject({
        code: 'storage_closed',
      });
      await expect(store.records.put(RECORDS.first)).rejects.toMatchObject({
        code: 'storage_closed',
      });
      await expect(store.schemaMetadata.getSchemaMeta()).rejects.toMatchObject({
        code: 'storage_closed',
      });
      await expect(store.withTransaction(() => undefined)).rejects.toMatchObject({
        code: 'storage_closed',
      });
    });

    it('serializes concurrent root transactions', async () => {
      const store = await makeStore();
      const events: string[] = [];
      const first = store.withTransaction(async (transaction) => {
        events.push('first:start');
        await transaction.keyValues.set('order', 'first');
        events.push('first:end');
      });
      const second = store.withTransaction(async (transaction) => {
        events.push('second:start');
        expect(await transaction.keyValues.get('order')).toBe('first');
        await transaction.keyValues.set('order', 'second');
        events.push('second:end');
      });
      await Promise.all([first, second]);
      expect(events).toEqual(['first:start', 'first:end', 'second:start', 'second:end']);
      expect(await store.keyValues.get('order')).toBe('second');
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

/** Adapter seam for identity-transition tests that must reopen named stores. */
export interface ScopedPersistenceContractAdapter {
  open(storeName: string): Promise<TransactionalStorageStore<ContractTestRecord>>;
}

interface ContractPersistence {
  readonly store: TransactionalStorageStore<ContractTestRecord>;
  close(): Promise<void>;
}

const identityDigest = (value: string): Promise<string> =>
  Promise.resolve(value.endsWith(':account-e') ? 'e'.repeat(64) : 'f'.repeat(64));

/**
 * Registers the E -> F -> E ownership suite against an adapter capable of
 * reopening independent named databases.
 */
export function describeScopedPersistenceContract(
  makeAdapter: ContractStoreFactory<ScopedPersistenceContractAdapter>,
): void {
  describe('authenticated scoped persistence contract', () => {
    it('isolates records and outbox writes across an offline E -> F -> E switch', async () => {
      const adapter = await makeAdapter();
      const events: string[] = [];
      const memory = {
        syncStatus: 'idle',
        pendingMutationView: 0,
        queryCacheEntries: 0,
        analyticsIdentity: undefined as string | undefined,
      };
      // No server hook is supplied: selecting either retained partition must stay fully local.
      const lifecycle = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'contract-app',
        registry: new InMemoryScopedPersistenceRegistryStore(),
        digest: identityDigest,
        open: async ({ storeName, subject }) => {
          events.push(`open:${subject}`);
          const store = await adapter.open(storeName);
          return {
            store,
            close: async () => {
              events.push(`close:start:${subject}`);
              await store.close();
              events.push(`close:end:${subject}`);
            },
          };
        },
        resetUserScopedState: () => {
          events.push('reset');
          memory.syncStatus = 'idle';
          memory.pendingMutationView = 0;
          memory.queryCacheEntries = 0;
          memory.analyticsIdentity = undefined;
        },
      });

      const accountE = await lifecycle.selectSubject('account-e');
      expect(accountE).toBeDefined();
      await accountE?.persistence.store.withTransaction(async (transaction) => {
        await transaction.records.put({ id: 'shared', label: 'account E', payload: 1 });
        await transaction.keyValues.set('outbox:pending', {
          entityId: 'shared',
          operation: 'put-e',
        });
      });
      memory.syncStatus = 'pending';
      memory.pendingMutationView = 1;
      memory.queryCacheEntries = 2;
      memory.analyticsIdentity = 'account-e';

      const accountF = await lifecycle.selectSubject('account-f');
      expect(accountF).toBeDefined();
      expect(await accountF?.persistence.store.records.get('shared')).toBeUndefined();
      expect(await accountF?.persistence.store.keyValues.get('outbox:pending')).toBeUndefined();
      expect(memory).toEqual({
        syncStatus: 'idle',
        pendingMutationView: 0,
        queryCacheEntries: 0,
        analyticsIdentity: undefined,
      });
      await accountF?.persistence.store.withTransaction(async (transaction) => {
        await transaction.records.put({ id: 'shared', label: 'account F', payload: 2 });
        await transaction.keyValues.set('outbox:pending', {
          entityId: 'shared',
          operation: 'put-f',
        });
      });

      const accountEReturned = await lifecycle.selectSubject('account-e');
      expect(await accountEReturned?.persistence.store.records.get('shared')).toMatchObject({
        label: 'account E',
      });
      expect(await accountEReturned?.persistence.store.keyValues.get('outbox:pending')).toEqual({
        entityId: 'shared',
        operation: 'put-e',
      });
      expect(events.indexOf('close:end:account-e')).toBeLessThan(events.indexOf('open:account-f'));
      expect(events.indexOf('close:end:account-f')).toBeLessThan(
        events.lastIndexOf('open:account-e'),
      );

      await lifecycle.clear();
    });

    it('claims an explicitly unowned legacy store once and rejects implicit cross-account use', async () => {
      const adapter = await makeAdapter();
      const legacy = await adapter.open('legacy-store');
      await legacy.records.put({ id: 'legacy', label: 'legacy E', payload: null });
      await legacy.close();
      const registry = new InMemoryScopedPersistenceRegistryStore();
      const lifecycle = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'legacy-contract',
        registry,
        digest: identityDigest,
        legacyStoreName: 'legacy-store',
        inspectLegacy: () => Promise.resolve({ exists: true, ownership: 'claimable' }),
        open: async ({ storeName }) => {
          const store = await adapter.open(storeName);
          return { store, close: () => store.close() };
        },
        resetUserScopedState: () => undefined,
      });

      const accountE = await lifecycle.selectSubject('account-e');
      expect(accountE?.storeName).toBe('legacy-store');
      expect(await accountE?.persistence.store.records.get('legacy')).toMatchObject({
        label: 'legacy E',
      });
      const accountF = await lifecycle.selectSubject('account-f');
      expect(accountF?.storeName).not.toBe('legacy-store');
      expect(await accountF?.persistence.store.records.get('legacy')).toBeUndefined();
      await lifecycle.clear();

      const otherOwned = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'other-owned-contract',
        registry: new InMemoryScopedPersistenceRegistryStore(),
        digest: identityDigest,
        legacyStoreName: 'legacy-store',
        inspectLegacy: () => Promise.resolve({ exists: true, ownership: 'other-subject' }),
        open: async ({ storeName }) => {
          const store = await adapter.open(storeName);
          return { store, close: () => store.close() };
        },
        resetUserScopedState: () => undefined,
      });
      const blockedClaim = await otherOwned.selectSubject('account-f');
      expect(blockedClaim?.storeName).not.toBe('legacy-store');
      expect(await blockedClaim?.persistence.store.records.get('legacy')).toBeUndefined();
      await otherOwned.clear();
    });

    it('blocks corrupt registry metadata before opening a domain store', async () => {
      const adapter = await makeAdapter();
      let opens = 0;
      const lifecycle = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'contract-app',
        registry: new InMemoryScopedPersistenceRegistryStore('{truncated'),
        digest: identityDigest,
        open: async ({ storeName }) => {
          opens += 1;
          const store = await adapter.open(storeName);
          return { store, close: () => store.close() };
        },
        resetUserScopedState: () => undefined,
      });

      await expect(lifecycle.selectSubject('account-e')).rejects.toMatchObject({
        code: 'persistence_identity_mismatch',
      });
      expect(opens).toBe(0);
      expect(lifecycle.current()).toBeUndefined();
      expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'identity-mismatch' });
    });

    it('leaves local data unchanged when the server subject fails the adoption recheck', async () => {
      const adapter = await makeAdapter();
      const lifecycle = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'contract-app',
        registry: new InMemoryScopedPersistenceRegistryStore(),
        digest: identityDigest,
        open: async ({ storeName }) => {
          const store = await adapter.open(storeName);
          return { store, close: () => store.close() };
        },
        resetUserScopedState: () => undefined,
      });
      const accountE = await lifecycle.selectSubject('account-e');
      await accountE?.persistence.store.records.put({
        id: 'preserved',
        label: 'before adoption',
        payload: true,
      });
      const adopt = async (): Promise<void> => {
        await accountE?.persistence.store.records.put({
          id: 'preserved',
          label: 'after adoption',
          payload: false,
        });
      };

      await expect(
        recheckServerSubjectBeforeSyncAdoption({
          partitionSubject: 'account-e',
          readServerSubject: () => Promise.resolve('account-f'),
          adopt,
        }),
      ).rejects.toMatchObject({ code: 'persistence_identity_mismatch' });
      expect(await accountE?.persistence.store.records.get('preserved')).toMatchObject({
        label: 'before adoption',
      });
      await lifecycle.clear();
    });

    it('closes and blocks the active partition on terminal session expiry', async () => {
      const adapter = await makeAdapter();
      let opens = 0;
      const lifecycle = new ScopedPersistenceLifecycle<ContractPersistence>({
        namespace: 'contract-app',
        registry: new InMemoryScopedPersistenceRegistryStore(),
        digest: identityDigest,
        open: async ({ storeName }) => {
          opens += 1;
          const store = await adapter.open(storeName);
          return { store, close: () => store.close() };
        },
        resetUserScopedState: () => undefined,
      });
      await lifecycle.selectSubject('account-e');

      await lifecycle.handleSessionExpired();

      expect(opens).toBe(1);
      expect(lifecycle.current()).toBeUndefined();
      expect(lifecycle.state).toMatchObject({ status: 'blocked', reason: 'session-expired' });
    });
  });
}
