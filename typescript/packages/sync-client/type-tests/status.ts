import { SyncStatusStore, type SyncAttentionItem } from '@baukit/sync-client';

type ProductAttention = SyncAttentionItem<{
  objectEntityType: string;
  objectEntityId: string;
  reasons: readonly string[];
}>;

const store = new SyncStatusStore<ProductAttention>();
store.setSyncing('2026-08-22T10:00:00Z');
store.setFailure({ kind: 'network' }, 'offline', {
  pendingCount: 1,
  retryAt: '2026-08-22T10:01:00Z',
});
store.setAttention(
  [
    {
      objectEntityType: 'workout_sessions',
      objectEntityId: 'session-1',
      reasons: ['future_server_rule'],
    },
  ],
  1,
);
const snapshot = store.getSnapshot();
const successAt: string | null = snapshot.lastSuccessAt;

export { snapshot, store, successAt };
