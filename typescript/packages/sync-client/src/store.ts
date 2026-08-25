/** Sync activity, as named by the offline-readiness contract. */
export type SyncStatus = 'idle' | 'syncing' | 'pending' | 'attention' | 'auth' | 'error';

/** Progress of the initial pull that a cold start must complete. */
export type InitialPullStatus = 'uninitialized' | 'pulling' | 'settled';

/** Readiness of the local store, derived from pull progress and delivery. */
export type LocalStoreReadiness =
  'uninitialized' | 'pulling' | 'hydrated-empty' | 'hydrated-populated';

/** Initial readiness as the offline-readiness contract names it. */
export type InitialSyncState = 'unknown' | 'syncing' | 'offline-cached' | 'settled';

/**
 * One rejection that needs product or user action. `entityType` and `entityId`
 * are opaque to baukit; products name their own entities.
 */
export type SyncAttentionItem<
  T = {
    entityType: string;
    entityId: string;
  },
> = T;

export interface SyncStatusSnapshot<TAttention = SyncAttentionItem> {
  status: SyncStatus;
  lastSyncAt: string | null;
  error: string | null;
  attention: readonly TAttention[];
  pendingCount: number;
  initialPullStatus: InitialPullStatus;
  /** Increments whenever a run delivers a new authoritative snapshot. */
  refreshRevision: number;
  /** Set when identity or corruption makes the local store unusable. */
  securityBlock: string | null;
}

export interface LocalStoreReadinessInput {
  initialPullStatus: InitialPullStatus;
  refreshRevision: number;
  deliveredRevision: number | null;
  hasData: boolean;
}

export interface SnakeCaseSyncStatusSnapshot<TAttention = SyncAttentionItem> {
  status: SyncStatus;
  last_sync_at: string | null;
  error: string | null;
  attention: readonly TAttention[];
  pending_count: number;
  initial_pull_status: InitialPullStatus;
  refresh_revision: number;
  security_block: string | null;
}

function initialSnapshot<TAttention>(): SyncStatusSnapshot<TAttention> {
  return {
    status: 'idle',
    lastSyncAt: null,
    error: null,
    attention: [],
    pendingCount: 0,
    initialPullStatus: 'uninitialized',
    refreshRevision: 0,
    securityBlock: null,
  };
}

/** Projects a status snapshot onto API-style snake_case field names. */
export function toSnakeCaseSnapshot<TAttention>(
  snapshot: SyncStatusSnapshot<TAttention>,
): SnakeCaseSyncStatusSnapshot<TAttention> {
  return {
    status: snapshot.status,
    last_sync_at: snapshot.lastSyncAt,
    error: snapshot.error,
    attention: snapshot.attention,
    pending_count: snapshot.pendingCount,
    initial_pull_status: snapshot.initialPullStatus,
    refresh_revision: snapshot.refreshRevision,
    security_block: snapshot.securityBlock,
  };
}

/**
 * Maps pull progress and snapshot delivery onto local-store readiness.
 *
 * A consumer that has not yet received the newest `refreshRevision` still reads
 * stale data, so it stays `pulling` rather than claiming a settled empty state.
 */
export function deriveLocalStoreReadiness({
  initialPullStatus,
  refreshRevision,
  deliveredRevision,
  hasData,
}: LocalStoreReadinessInput): LocalStoreReadiness {
  if (initialPullStatus === 'uninitialized') return 'uninitialized';
  if (initialPullStatus === 'pulling' || deliveredRevision !== refreshRevision) return 'pulling';
  return hasData ? 'hydrated-populated' : 'hydrated-empty';
}

/** Maps a snapshot onto the contract's initial-readiness vocabulary. */
export function deriveInitialSyncState({
  initialPullStatus,
  lastSyncAt,
  status,
}: Pick<SyncStatusSnapshot, 'initialPullStatus' | 'lastSyncAt' | 'status'>): InitialSyncState {
  if (initialPullStatus === 'uninitialized') return 'unknown';
  if (initialPullStatus === 'pulling') return 'syncing';
  if (status === 'error' && lastSyncAt === null) return 'offline-cached';
  return 'settled';
}

function settledStatus(attentionCount: number, pendingCount: number): SyncStatus {
  if (attentionCount > 0) return 'attention';
  return pendingCount > 0 ? 'pending' : 'idle';
}

/**
 * Observable sync status, with no React and no state library.
 *
 * Products subscribe directly, or adapt {@link SyncStatusStore.subscribe} and
 * {@link SyncStatusStore.getSnapshot} to whatever their UI layer expects.
 */
export class SyncStatusStore<TAttention = SyncAttentionItem> {
  private snapshot: SyncStatusSnapshot<TAttention> = initialSnapshot();
  private readonly listeners = new Set<(snapshot: SyncStatusSnapshot<TAttention>) => void>();

  getSnapshot(): SyncStatusSnapshot<TAttention> {
    return this.snapshot;
  }

  subscribe(listener: (snapshot: SyncStatusSnapshot<TAttention>) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  setSyncing(): void {
    this.set((state) => ({
      status: 'syncing',
      error: null,
      initialPullStatus: state.lastSyncAt === null ? 'pulling' : 'settled',
    }));
  }

  setIdle(lastSyncAt: string): void {
    this.set((state) => ({
      status: 'idle',
      lastSyncAt,
      error: null,
      attention: [],
      pendingCount: 0,
      initialPullStatus: 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  setAttention(items: readonly TAttention[], pendingCount: number): void {
    this.set((state) => ({
      status: items.length > 0 ? 'attention' : 'pending',
      error: null,
      attention: [...items],
      pendingCount,
      initialPullStatus: 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  setAuth(message: string): void {
    this.set((state) => ({
      status: 'auth',
      error: message,
      initialPullStatus: 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  setError(message: string, pendingCount?: number): void {
    this.set((state) => {
      const nextPendingCount = pendingCount ?? state.pendingCount;
      return {
        status:
          state.attention.length > 0 ? 'attention' : nextPendingCount > 0 ? 'pending' : 'error',
        error: message,
        pendingCount: nextPendingCount,
        initialPullStatus: 'settled',
        refreshRevision: state.refreshRevision + 1,
      };
    });
  }

  /** Restores persisted status on a cold start, before the first run. */
  hydrate(lastSyncAt: string | null, items: readonly TAttention[] = [], pendingCount = 0): void {
    this.set((state) => ({
      status: settledStatus(items.length, pendingCount),
      lastSyncAt,
      error: null,
      attention: [...items],
      pendingCount,
      initialPullStatus: lastSyncAt === null ? 'uninitialized' : 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  reset(): void {
    this.set(() => initialSnapshot());
  }

  setSecurityBlock(message: string): void {
    this.set(() => ({
      status: 'error',
      error: message,
      initialPullStatus: 'settled',
      securityBlock: message,
    }));
  }

  resumeAfterAuth(): void {
    this.set((state) => ({
      status: settledStatus(state.attention.length, state.pendingCount),
      error: null,
    }));
  }

  private set(
    update: (state: SyncStatusSnapshot<TAttention>) => Partial<SyncStatusSnapshot<TAttention>>,
  ): void {
    this.snapshot = { ...this.snapshot, ...update(this.snapshot) };
    this.listeners.forEach((listener) => {
      listener(this.snapshot);
    });
  }
}

/** Write-only view a sync run uses to report progress. */
export interface SyncStatusSink<TAttention = SyncAttentionItem> {
  setSyncing(): void;
  setIdle(lastSyncAt: string): void;
  setAttention(items: readonly TAttention[], pendingCount: number): void;
  setAuth(message: string): void;
  setError(message: string, pendingCount?: number): void;
}
