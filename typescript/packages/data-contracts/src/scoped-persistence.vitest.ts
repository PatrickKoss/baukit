import { describe, expect, it } from 'vitest';

import type { TransactionalStorageStore } from './contracts.js';
import {
  InMemoryScopedPersistenceRegistryStore,
  recheckServerSubjectBeforeSyncAdoption,
} from './identity.js';
import { ScopedPersistenceLifecycle } from './scoped-lifecycle.js';
import type { ContractStoreFactory, ContractTestRecord } from './vitest.js';

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
    describeOfflineSwitchIsolation(makeAdapter);
    describeCorruptRegistryBlocking(makeAdapter);
    describeAdoptionRecheck(makeAdapter);
    describeSessionExpiryBlocking(makeAdapter);
  });
}

async function writeSharedWithOutbox(
  store: TransactionalStorageStore<ContractTestRecord> | undefined,
  label: string,
  payload: number,
  operation: string,
): Promise<void> {
  await store?.withTransaction(async (transaction) => {
    await transaction.records.put({ id: 'shared', label, payload });
    await transaction.keyValues.set('outbox:pending', { entityId: 'shared', operation });
  });
}

function describeOfflineSwitchIsolation(
  makeAdapter: ContractStoreFactory<ScopedPersistenceContractAdapter>,
): void {
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
    await writeSharedWithOutbox(accountE?.persistence.store, 'account E', 1, 'put-e');
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
    await writeSharedWithOutbox(accountF?.persistence.store, 'account F', 2, 'put-f');

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
}

function describeCorruptRegistryBlocking(
  makeAdapter: ContractStoreFactory<ScopedPersistenceContractAdapter>,
): void {
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
}

function describeAdoptionRecheck(
  makeAdapter: ContractStoreFactory<ScopedPersistenceContractAdapter>,
): void {
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
}

function describeSessionExpiryBlocking(
  makeAdapter: ContractStoreFactory<ScopedPersistenceContractAdapter>,
): void {
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
}
