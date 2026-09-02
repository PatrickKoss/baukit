import {
  SyncStatusStore,
  toSnakeCaseSnapshot,
  type SnakeCaseSyncStatusSnapshot,
  type SyncAttentionItem,
} from '@baukit/sync-client';

type ProductAttention = SyncAttentionItem<{
  object_entity_type: string;
  object_entity_id: string;
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
      object_entity_type: 'workout_sessions',
      object_entity_id: 'session-1',
      reasons: ['future_server_rule'],
    },
  ],
  1,
);
const snapshot: SnakeCaseSyncStatusSnapshot<ProductAttention> = toSnakeCaseSnapshot(
  store.getSnapshot(),
);
const successAt: string | null = snapshot.last_success_at;

export { snapshot, store, successAt };
