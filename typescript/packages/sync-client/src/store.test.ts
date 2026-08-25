import { describe, expect, it, vi } from 'vitest';

import {
  deriveInitialSyncState,
  deriveLocalStoreReadiness,
  SyncStatusStore,
  toSnakeCaseSnapshot,
  type SyncAttentionItem,
} from './store.js';

const conflict: SyncAttentionItem = { entityType: 'parent', entityId: 'parent-1' };

describe('SyncStatusStore', () => {
  it('starts uninitialized and idle', () => {
    expect(new SyncStatusStore().getSnapshot()).toEqual({
      status: 'idle',
      lastSyncAt: null,
      error: null,
      attention: [],
      pendingCount: 0,
      initialPullStatus: 'uninitialized',
      refreshRevision: 0,
      securityBlock: null,
    });
  });

  it('treats the first run as the initial pull and later runs as settled', () => {
    const store = new SyncStatusStore();

    store.setSyncing();
    expect(store.getSnapshot()).toMatchObject({ status: 'syncing', initialPullStatus: 'pulling' });

    store.setIdle('2026-08-22T10:00:00Z');
    store.setSyncing();
    expect(store.getSnapshot()).toMatchObject({ status: 'syncing', initialPullStatus: 'settled' });
  });

  it('clears attention and pending work on a successful run', () => {
    const store = new SyncStatusStore();
    store.setAttention([conflict], 3);

    store.setIdle('2026-08-22T10:00:00Z');

    expect(store.getSnapshot()).toMatchObject({
      status: 'idle',
      lastSyncAt: '2026-08-22T10:00:00Z',
      attention: [],
      pendingCount: 0,
      error: null,
    });
  });

  it('reports pending when work remains but nothing is actionable', () => {
    const store = new SyncStatusStore();

    store.setAttention([], 2);

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', pendingCount: 2 });
  });

  it('reports attention when an actionable rejection exists', () => {
    const store = new SyncStatusStore();

    store.setAttention([conflict], 1);

    expect(store.getSnapshot()).toMatchObject({ status: 'attention', attention: [conflict] });
  });

  it('keeps a product-defined attention item intact', () => {
    type ProductAttention = SyncAttentionItem<{
      object_entity_type: string;
      object_entity_id: string;
      reasons: readonly string[];
    }>;
    const item: ProductAttention = {
      object_entity_type: 'workout_sessions',
      object_entity_id: 'session-1',
      reasons: ['future_server_rule'],
    };
    const store = new SyncStatusStore<ProductAttention>();

    store.setAttention([item], 1);

    expect(store.getSnapshot().attention).toEqual([item]);
  });

  it('keeps attention ahead of a transport failure', () => {
    const store = new SyncStatusStore();
    store.setAttention([conflict], 1);

    store.setError('network down');

    expect(store.getSnapshot()).toMatchObject({ status: 'attention', error: 'network down' });
  });

  it('reports pending rather than error while unsent work remains', () => {
    const store = new SyncStatusStore();

    store.setError('network down', 4);

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', pendingCount: 4 });
  });

  it('reports error only when nothing is pending or actionable', () => {
    const store = new SyncStatusStore();

    store.setError('network down', 0);

    expect(store.getSnapshot()).toMatchObject({ status: 'error', pendingCount: 0 });
  });

  it('keeps the previous pending count when setError omits one', () => {
    const store = new SyncStatusStore();
    store.setAttention([], 5);

    store.setError('network down');

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', pendingCount: 5 });
  });

  it('resumes the settled status after re-authentication', () => {
    const store = new SyncStatusStore();
    store.setAttention([], 2);
    store.setAuth('sign in again');
    expect(store.getSnapshot()).toMatchObject({ status: 'auth', error: 'sign in again' });

    store.resumeAfterAuth();

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', error: null });
  });

  it('hydrates a cold start from persisted status', () => {
    const store = new SyncStatusStore();

    store.hydrate('2026-08-22T09:00:00Z', [conflict], 1);

    expect(store.getSnapshot()).toMatchObject({
      status: 'attention',
      lastSyncAt: '2026-08-22T09:00:00Z',
      initialPullStatus: 'settled',
    });
  });

  it('stays uninitialized when hydrating a store that never synced', () => {
    const store = new SyncStatusStore();

    store.hydrate(null);

    expect(store.getSnapshot()).toMatchObject({
      status: 'idle',
      initialPullStatus: 'uninitialized',
    });
  });

  it('records a security block without losing the message', () => {
    const store = new SyncStatusStore();

    store.setSecurityBlock('database belongs to another account');

    expect(store.getSnapshot()).toMatchObject({
      status: 'error',
      securityBlock: 'database belongs to another account',
    });
  });

  it('reset returns the initial snapshot', () => {
    const store = new SyncStatusStore();
    store.setAttention([conflict], 3);

    store.reset();

    expect(store.getSnapshot()).toMatchObject({
      status: 'idle',
      attention: [],
      refreshRevision: 0,
    });
  });

  it('bumps refreshRevision on every delivered snapshot', () => {
    const store = new SyncStatusStore();

    store.setIdle('2026-08-22T10:00:00Z');
    store.setAttention([], 1);

    expect(store.getSnapshot().refreshRevision).toBe(2);
  });

  it('does not bump refreshRevision when a run merely starts', () => {
    const store = new SyncStatusStore();

    store.setSyncing();

    expect(store.getSnapshot().refreshRevision).toBe(0);
  });

  it('notifies subscribers until they unsubscribe', () => {
    const store = new SyncStatusStore();
    const listener = vi.fn();
    const unsubscribe = store.subscribe(listener);

    store.setSyncing();
    expect(listener).toHaveBeenCalledTimes(1);
    expect(listener).toHaveBeenCalledWith(store.getSnapshot());

    unsubscribe();
    store.setIdle('2026-08-22T10:00:00Z');
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe('toSnakeCaseSnapshot', () => {
  it('projects every status field without changing attention items', () => {
    const store = new SyncStatusStore<{ id: string }>();
    store.hydrate('2026-08-22T09:00:00Z', [{ id: 'conflict-1' }], 2);

    expect(toSnakeCaseSnapshot(store.getSnapshot())).toEqual({
      status: 'attention',
      last_sync_at: '2026-08-22T09:00:00Z',
      error: null,
      attention: [{ id: 'conflict-1' }],
      pending_count: 2,
      initial_pull_status: 'settled',
      refresh_revision: 1,
      security_block: null,
    });
  });
});

describe('deriveLocalStoreReadiness', () => {
  it('stays uninitialized before the first pull starts', () => {
    expect(
      deriveLocalStoreReadiness({
        initialPullStatus: 'uninitialized',
        refreshRevision: 0,
        deliveredRevision: 0,
        hasData: true,
      }),
    ).toBe('uninitialized');
  });

  it('stays pulling while the initial pull runs', () => {
    expect(
      deriveLocalStoreReadiness({
        initialPullStatus: 'pulling',
        refreshRevision: 1,
        deliveredRevision: 1,
        hasData: false,
      }),
    ).toBe('pulling');
  });

  it('stays pulling while the consumer still holds a stale snapshot', () => {
    expect(
      deriveLocalStoreReadiness({
        initialPullStatus: 'settled',
        refreshRevision: 2,
        deliveredRevision: 1,
        hasData: false,
      }),
    ).toBe('pulling');
  });

  it('reports an empty state only once the newest snapshot is delivered', () => {
    expect(
      deriveLocalStoreReadiness({
        initialPullStatus: 'settled',
        refreshRevision: 2,
        deliveredRevision: 2,
        hasData: false,
      }),
    ).toBe('hydrated-empty');
  });

  it('reports a populated store', () => {
    expect(
      deriveLocalStoreReadiness({
        initialPullStatus: 'settled',
        refreshRevision: 2,
        deliveredRevision: 2,
        hasData: true,
      }),
    ).toBe('hydrated-populated');
  });
});

describe('deriveInitialSyncState', () => {
  it('maps an unstarted pull to unknown', () => {
    expect(
      deriveInitialSyncState({
        initialPullStatus: 'uninitialized',
        lastSyncAt: null,
        status: 'idle',
      }),
    ).toBe('unknown');
  });

  it('maps a running initial pull to syncing', () => {
    expect(
      deriveInitialSyncState({ initialPullStatus: 'pulling', lastSyncAt: null, status: 'syncing' }),
    ).toBe('syncing');
  });

  it('maps a first-run failure to offline-cached', () => {
    expect(
      deriveInitialSyncState({ initialPullStatus: 'settled', lastSyncAt: null, status: 'error' }),
    ).toBe('offline-cached');
  });

  it('maps a failure after a past success to settled', () => {
    expect(
      deriveInitialSyncState({
        initialPullStatus: 'settled',
        lastSyncAt: '2026-08-22T09:00:00Z',
        status: 'error',
      }),
    ).toBe('settled');
  });
});
