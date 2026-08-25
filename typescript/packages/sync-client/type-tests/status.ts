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

export { snapshot, store };
