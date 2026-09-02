import { describe, expect, it, vi } from 'vitest';

import {
  deriveInitialSyncState,
  deriveLocalStoreReadiness,
  SyncStatusStore,
  toSnakeCaseSnapshot,
  type SyncAttentionItem,
} from './store.js';

const conflict: SyncAttentionItem = { entityType: 'parent', entityId: 'parent-1' };
const firstAttemptAt = '2026-08-22T09:59:00Z';
const firstSuccessAt = '2026-08-22T10:00:00Z';
const failedAttemptAt = '2026-08-22T10:05:00Z';

describe('SyncStatusStore', () => {
  it('starts uninitialized and idle', () => {
    expect(new SyncStatusStore().getSnapshot()).toEqual({
      status: 'idle',
      lastAttemptAt: null,
      lastSuccessAt: null,
      lastSyncAt: null,
      error: null,
      failure: null,
      retrying: false,
      retryAt: null,
      attention: [],
      pendingCount: 0,
      initialPullStatus: 'uninitialized',
      refreshRevision: 0,
      securityBlock: null,
    });
  });

  it('treats the first run as the initial pull and later runs as settled', () => {
    const store = new SyncStatusStore();

    store.setSyncing(firstAttemptAt);
    expect(store.getSnapshot()).toMatchObject({ status: 'syncing', initialPullStatus: 'pulling' });

    store.setIdle(firstSuccessAt);
    store.setSyncing(failedAttemptAt);
    expect(store.getSnapshot()).toMatchObject({ status: 'syncing', initialPullStatus: 'settled' });
  });

  it('clears attention and pending work on a successful run', () => {
    const store = new SyncStatusStore();
    store.setAttention([conflict], 3);

    store.setIdle(firstSuccessAt);

    expect(store.getSnapshot()).toMatchObject({
      status: 'idle',
      lastAttemptAt: firstSuccessAt,
      lastSuccessAt: firstSuccessAt,
      lastSyncAt: firstSuccessAt,
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

    store.setFailure({ kind: 'network' }, 'network down');

    expect(store.getSnapshot()).toMatchObject({ status: 'attention', error: 'network down' });
  });

  it('reports pending rather than error while unsent work remains', () => {
    const store = new SyncStatusStore();

    store.setFailure({ kind: 'network' }, 'network down', { pendingCount: 4 });

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', pendingCount: 4 });
  });

  it('reports error only when nothing is pending or actionable', () => {
    const store = new SyncStatusStore();

    store.setFailure({ kind: 'network' }, 'network down', { pendingCount: 0 });

    expect(store.getSnapshot()).toMatchObject({ status: 'error', pendingCount: 0 });
  });

  it('keeps the previous pending count when setFailure omits one', () => {
    const store = new SyncStatusStore();
    store.setAttention([], 5);

    store.setFailure({ kind: 'network' }, 'network down');

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', pendingCount: 5 });
  });

  it('advances only the attempt timestamp when a retry fails', () => {
    const store = new SyncStatusStore();
    store.setIdle(firstSuccessAt);
    store.setSyncing(failedAttemptAt);

    store.setFailure({ kind: 'network' }, 'offline', { attemptAt: failedAttemptAt });

    expect(store.getSnapshot()).toMatchObject({
      lastAttemptAt: failedAttemptAt,
      lastSuccessAt: firstSuccessAt,
      lastSyncAt: firstSuccessAt,
      failure: { kind: 'network' },
    });
  });

  it.each(['network', 'server', 'local_apply'] as const)(
    'keeps pending work visible after a %s failure',
    (kind) => {
      const store = new SyncStatusStore();
      store.setAttention([], 4);

      store.setFailure({ kind }, 'run failed', { attemptAt: failedAttemptAt });

      expect(store.getSnapshot()).toMatchObject({
        status: 'pending',
        pendingCount: 4,
        failure: { kind },
        lastSuccessAt: null,
      });
    },
  );

  it('exposes a rate limit and its scheduled retry without reporting offline', () => {
    const store = new SyncStatusStore();
    const retryAt = '2026-08-22T10:06:00Z';
    store.setSyncing(firstAttemptAt);

    store.setFailure({ kind: 'rate_limited', retryAt }, 'too many requests', {
      attemptAt: failedAttemptAt,
    });

    expect(store.getSnapshot()).toMatchObject({
      failure: { kind: 'rate_limited', retryAt },
      retrying: true,
      retryAt,
    });
    expect(deriveInitialSyncState(store.getSnapshot())).toBe('sync-error-cached');
  });

  it('records a scheduled network retry separately from its failure', () => {
    const store = new SyncStatusStore();
    store.setFailure({ kind: 'network' }, 'offline', {
      pendingCount: 2,
      attemptAt: failedAttemptAt,
    });

    store.setRetrying('2026-08-22T10:06:00Z');

    expect(store.getSnapshot()).toMatchObject({
      status: 'pending',
      failure: { kind: 'network' },
      retrying: true,
      retryAt: '2026-08-22T10:06:00Z',
      pendingCount: 2,
    });
  });

  it('resumes the settled status after re-authentication', () => {
    const store = new SyncStatusStore();
    store.setAttention([], 2);
    store.setAuth('sign in again');
    expect(store.getSnapshot()).toMatchObject({
      status: 'auth',
      error: 'sign in again',
      failure: { kind: 'auth' },
    });

    store.resumeAfterAuth();

    expect(store.getSnapshot()).toMatchObject({ status: 'pending', error: null });
  });

  it('hydrates a cold start from persisted status', () => {
    const store = new SyncStatusStore();

    store.hydrate({
      lastAttemptAt: '2026-08-22T09:01:00Z',
      lastSuccessAt: '2026-08-22T09:00:00Z',
      attention: [conflict],
      pendingCount: 1,
    });

    expect(store.getSnapshot()).toMatchObject({
      status: 'attention',
      lastAttemptAt: '2026-08-22T09:01:00Z',
      lastSuccessAt: '2026-08-22T09:00:00Z',
      lastSyncAt: '2026-08-22T09:00:00Z',
      initialPullStatus: 'settled',
    });
  });

  it('stays uninitialized when hydrating a store that never synced', () => {
    const store = new SyncStatusStore();

    store.hydrate({ lastAttemptAt: null, lastSuccessAt: null });

    expect(store.getSnapshot()).toMatchObject({
      status: 'idle',
      initialPullStatus: 'uninitialized',
    });
  });

  it('records a security block without losing the message', () => {
    const store = new SyncStatusStore();

    store.setSecurityBlock('database belongs to another account', {
      kind: 'partition_mismatch',
    });

    expect(store.getSnapshot()).toMatchObject({
      status: 'error',
      failure: { kind: 'partition_mismatch' },
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

    store.setIdle(firstSuccessAt);
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
    store.setIdle(firstSuccessAt);
    expect(listener).toHaveBeenCalledTimes(1);
  });
});

describe('toSnakeCaseSnapshot', () => {
  it('projects every status field without changing attention items', () => {
    const store = new SyncStatusStore<{ id: string }>();
    store.hydrate({
      lastAttemptAt: '2026-08-22T09:02:00Z',
      lastSuccessAt: '2026-08-22T09:00:00Z',
      attention: [{ id: 'conflict-1' }],
      pendingCount: 2,
    });

    expect(toSnakeCaseSnapshot(store.getSnapshot())).toEqual({
      status: 'attention',
      last_attempt_at: '2026-08-22T09:02:00Z',
      last_success_at: '2026-08-22T09:00:00Z',
      last_sync_at: '2026-08-22T09:00:00Z',
      error: null,
      failure: null,
      retrying: false,
      retry_at: null,
      attention: [{ id: 'conflict-1' }],
      pending_count: 2,
      initial_pull_status: 'settled',
      refresh_revision: 1,
      security_block: null,
    });
  });

  it('projects rate-limit metadata without product copy', () => {
    const store = new SyncStatusStore();
    store.setFailure({ kind: 'rate_limited', retryAt: '2026-08-22T10:05:00Z' }, 'product message', {
      attemptAt: failedAttemptAt,
    });

    expect(toSnakeCaseSnapshot(store.getSnapshot())).toMatchObject({
      error: 'product message',
      failure: { kind: 'rate_limited', retry_at: '2026-08-22T10:05:00Z' },
      retrying: true,
      retry_at: '2026-08-22T10:05:00Z',
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
        lastSuccessAt: null,
        failure: null,
        status: 'idle',
      }),
    ).toBe('unknown');
  });

  it('maps a running initial pull to syncing', () => {
    expect(
      deriveInitialSyncState({
        initialPullStatus: 'pulling',
        lastSuccessAt: null,
        failure: null,
        status: 'syncing',
      }),
    ).toBe('syncing');
  });

  it('maps a first-run failure to offline-cached', () => {
    expect(
      deriveInitialSyncState({
        initialPullStatus: 'settled',
        lastSuccessAt: null,
        failure: { kind: 'network' },
        status: 'error',
      }),
    ).toBe('offline-cached');
  });

  it('maps a failure after a past success to settled', () => {
    expect(
      deriveInitialSyncState({
        initialPullStatus: 'settled',
        lastSuccessAt: '2026-08-22T09:00:00Z',
        failure: { kind: 'server' },
        status: 'error',
      }),
    ).toBe('settled');
  });
});
