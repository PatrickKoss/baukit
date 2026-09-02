import { toSnakeCaseFailure, type SnakeCaseSyncFailure, type SyncFailure } from './error.js';

/** Sync activity, as named by the offline-readiness contract. */
export type SyncStatus = 'idle' | 'syncing' | 'pending' | 'attention' | 'auth' | 'error';

/** Progress of the initial pull that a cold start must complete. */
export type InitialPullStatus = 'uninitialized' | 'pulling' | 'settled';

/** Readiness of the local store, derived from pull progress and delivery. */
export type LocalStoreReadiness =
  'uninitialized' | 'pulling' | 'hydrated-empty' | 'hydrated-populated';

/** Initial readiness as the offline-readiness contract names it. */
export type InitialSyncState =
  'unknown' | 'syncing' | 'offline-cached' | 'sync-error-cached' | 'settled';

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
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  /** @deprecated Use `lastSuccessAt`. This field will be removed after one release cycle. */
  lastSyncAt: string | null;
  error: string | null;
  failure: SyncFailure | null;
  retrying: boolean;
  retryAt: string | null;
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
  last_attempt_at: string | null;
  last_success_at: string | null;
  /** @deprecated Use `last_success_at`. This field will be removed after one release cycle. */
  last_sync_at: string | null;
  error: string | null;
  failure: SnakeCaseSyncFailure | null;
  retrying: boolean;
  retry_at: string | null;
  attention: readonly TAttention[];
  pending_count: number;
  initial_pull_status: InitialPullStatus;
  refresh_revision: number;
  security_block: string | null;
}

export interface SyncStatusHydration<TAttention = SyncAttentionItem> {
  lastAttemptAt: string | null;
  lastSuccessAt: string | null;
  attention?: readonly TAttention[];
  pendingCount?: number;
}

export interface SyncFailureUpdate {
  /** Defaults to the store's current pending count. */
  pendingCount?: number;
  /** Defaults to the current time. */
  attemptAt?: string;
  /** Marks a retry as scheduled. Rate-limit failures use their own `retryAt`. */
  retryAt?: string | null;
}

export interface SyncStatusStoreOptions {
  clock?: () => string;
}

function initialSnapshot<TAttention>(): SyncStatusSnapshot<TAttention> {
  return {
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
  };
}

/** Projects a status snapshot onto API-style snake_case field names. */
export function toSnakeCaseSnapshot<TAttention>(
  snapshot: SyncStatusSnapshot<TAttention>,
): SnakeCaseSyncStatusSnapshot<TAttention> {
  return {
    status: snapshot.status,
    last_attempt_at: snapshot.lastAttemptAt,
    last_success_at: snapshot.lastSuccessAt,
    last_sync_at: snapshot.lastSuccessAt,
    error: snapshot.error,
    failure: snapshot.failure === null ? null : toSnakeCaseFailure(snapshot.failure),
    retrying: snapshot.retrying,
    retry_at: snapshot.retryAt,
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
  lastSuccessAt,
  status,
  failure,
}: Pick<
  SyncStatusSnapshot,
  'failure' | 'initialPullStatus' | 'lastSuccessAt' | 'status'
>): InitialSyncState {
  if (initialPullStatus === 'uninitialized') return 'unknown';
  if (initialPullStatus === 'pulling') return 'syncing';
  if (lastSuccessAt === null && failure !== null) {
    return failure.kind === 'network' ? 'offline-cached' : 'sync-error-cached';
  }
  if (status === 'error' && lastSuccessAt === null) return 'sync-error-cached';
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
  private readonly clock: () => string;

  constructor(options: SyncStatusStoreOptions = {}) {
    this.clock = options.clock ?? (() => new Date().toISOString());
  }

  getSnapshot(): SyncStatusSnapshot<TAttention> {
    return this.snapshot;
  }

  subscribe(listener: (snapshot: SyncStatusSnapshot<TAttention>) => void): () => void {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  }

  setSyncing(attemptAt = this.clock()): void {
    this.set((state) => ({
      status: 'syncing',
      lastAttemptAt: attemptAt,
      error: null,
      retryAt: null,
      initialPullStatus: state.lastSuccessAt === null ? 'pulling' : 'settled',
    }));
  }

  setIdle(successAt: string): void {
    this.set((state) => ({
      status: 'idle',
      lastAttemptAt: successAt,
      lastSuccessAt: successAt,
      error: null,
      failure: null,
      retrying: false,
      retryAt: null,
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
      failure: null,
      retrying: false,
      retryAt: null,
      attention: [...items],
      pendingCount,
      initialPullStatus: 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  setAuth(message: string, update: SyncFailureUpdate = {}): void {
    this.setFailure({ kind: 'auth' }, message, update);
  }

  setFailure(failure: SyncFailure, message: string, update: SyncFailureUpdate = {}): void {
    this.set((state) => {
      const nextPendingCount = update.pendingCount ?? state.pendingCount;
      const retryAt = failure.kind === 'rate_limited' ? failure.retryAt : (update.retryAt ?? null);
      return {
        status:
          failure.kind === 'auth'
            ? 'auth'
            : state.attention.length > 0
              ? 'attention'
              : nextPendingCount > 0
                ? 'pending'
                : 'error',
        lastAttemptAt: update.attemptAt ?? this.clock(),
        error: message,
        failure,
        retrying: retryAt !== null,
        retryAt,
        pendingCount: nextPendingCount,
        initialPullStatus: 'settled',
        refreshRevision: state.refreshRevision + 1,
      };
    });
  }

  setRetrying(retryAt: string, pendingCount?: number): void {
    this.set((state) => {
      const nextPendingCount = pendingCount ?? state.pendingCount;
      return {
        status:
          state.attention.length > 0 ? 'attention' : nextPendingCount > 0 ? 'pending' : 'error',
        retrying: true,
        retryAt,
        pendingCount: nextPendingCount,
      };
    });
  }

  /** @deprecated Use `setFailure` with typed metadata. */
  setError(message: string, pendingCount?: number): void {
    this.setFailure(
      { kind: 'network' },
      message,
      pendingCount === undefined ? {} : { pendingCount },
    );
  }

  /** Restores persisted status on a cold start, before the first run. */
  hydrate(hydration: SyncStatusHydration<TAttention>): void;
  /** @deprecated Pass a `SyncStatusHydration` object with both timestamps. */
  hydrate(lastSyncAt: string | null, items?: readonly TAttention[], pendingCount?: number): void;
  hydrate(
    hydrationOrLastSyncAt: SyncStatusHydration<TAttention> | string | null,
    legacyItems: readonly TAttention[] = [],
    legacyPendingCount = 0,
  ): void {
    const hydration: SyncStatusHydration<TAttention> =
      typeof hydrationOrLastSyncAt === 'object' && hydrationOrLastSyncAt !== null
        ? hydrationOrLastSyncAt
        : {
            lastAttemptAt: hydrationOrLastSyncAt,
            lastSuccessAt: hydrationOrLastSyncAt,
            attention: legacyItems,
            pendingCount: legacyPendingCount,
          };
    const items = hydration.attention ?? [];
    const pendingCount = hydration.pendingCount ?? 0;
    this.set((state) => ({
      status: settledStatus(items.length, pendingCount),
      lastAttemptAt: hydration.lastAttemptAt,
      lastSuccessAt: hydration.lastSuccessAt,
      error: null,
      failure: null,
      retrying: false,
      retryAt: null,
      attention: [...items],
      pendingCount,
      initialPullStatus: hydration.lastSuccessAt === null ? 'uninitialized' : 'settled',
      refreshRevision: state.refreshRevision + 1,
    }));
  }

  reset(): void {
    this.set(() => initialSnapshot());
  }

  setSecurityBlock(
    message: string,
    failure: Extract<SyncFailure, { kind: 'partition_mismatch' | 'local_apply' }> = {
      kind: 'local_apply',
    },
  ): void {
    this.set(() => ({
      status: 'error',
      lastAttemptAt: this.clock(),
      error: message,
      failure,
      retrying: false,
      retryAt: null,
      initialPullStatus: 'settled',
      securityBlock: message,
    }));
  }

  resumeAfterAuth(): void {
    this.set((state) => ({
      status: settledStatus(state.attention.length, state.pendingCount),
      error: null,
      failure: null,
      retrying: false,
      retryAt: null,
    }));
  }

  private set(
    update: (state: SyncStatusSnapshot<TAttention>) => Partial<SyncStatusSnapshot<TAttention>>,
  ): void {
    const next = { ...this.snapshot, ...update(this.snapshot) };
    this.snapshot = { ...next, lastSyncAt: next.lastSuccessAt };
    this.listeners.forEach((listener) => {
      listener(this.snapshot);
    });
  }
}

/** Write-only view a sync run uses to report progress. */
export interface SyncStatusSink<TAttention = SyncAttentionItem> {
  setSyncing(attemptAt?: string): void;
  setIdle(successAt: string): void;
  setAttention(items: readonly TAttention[], pendingCount: number): void;
  setAuth(message: string, update?: SyncFailureUpdate): void;
  setFailure(failure: SyncFailure, message: string, update?: SyncFailureUpdate): void;
  setRetrying(retryAt: string, pendingCount?: number): void;
}
