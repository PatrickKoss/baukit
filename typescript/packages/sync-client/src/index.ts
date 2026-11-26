export {
  SyncAuthError,
  SyncLocalApplyError,
  SyncNetworkError,
  SyncPartitionMismatchError,
  SyncPayloadCompatibilityError,
  SyncRateLimitError,
  SyncServerError,
  SyncTransportError,
  syncFailureFromError,
} from './error.js';
export type { SyncFailure } from './error.js';
export {
  compareHybridLogicalTimestamps,
  DEFAULT_HLC_MAX_DRIFT_MS,
  decodeHybridLogicalTimestamp,
  encodeHybridLogicalTimestamp,
  HybridLogicalClock,
  HybridLogicalClockError,
  HLC_COUNTERS_PER_MILLISECOND,
  HLC_STORAGE_KEY,
  MAX_HLC_TIMESTAMP,
  nowStep,
  observeStep,
  validateClientTimestamp,
} from './hlc.js';
export type {
  HlcPhysicalClock,
  HlcStep,
  HlcStorage,
  HybridLogicalClockOptions,
  HybridLogicalClockErrorCode,
  HybridLogicalClockState,
} from './hlc.js';
export { dependencyRankByOrder, rankPushBatch, validatePushOutcomeCoverage } from './push-batch.js';
export type {
  PushCandidate,
  PushOutcomeCoverageOptions,
  RankedPushItem,
  RankPushBatchOptions,
} from './push-batch.js';
export { SyncScheduler } from './scheduler.js';
export type {
  SyncSchedulerEnvironment,
  SyncSchedulerOptions,
  SyncSchedulerRecoverySignal,
  SyncSchedulerRetry,
  SyncSchedulerRetryOptions,
  SyncSchedulerTimer,
} from './scheduler.js';
export { deriveInitialSyncState, deriveLocalStoreReadiness, SyncStatusStore } from './store.js';
export type {
  InitialPullStatus,
  InitialSyncState,
  LocalStoreReadiness,
  LocalStoreReadinessInput,
  SyncAttentionItem,
  SyncFailureUpdate,
  SyncStatus,
  SyncStatusHydration,
  SyncStatusSink,
  SyncStatusSnapshot,
  SyncStatusStoreOptions,
} from './store.js';
export {
  commitCursorAfterLocalTransaction,
  DEFAULT_REQUEST_TIMEOUT_MS,
  DEFAULT_RETRY_AFTER_FALLBACK_MS,
  parseRetryAfter,
  SyncTransport,
  validatePullPage,
} from './transport.js';
export type {
  CursorCommitOptions,
  ParseRetryAfterOptions,
  PullPagePosition,
  SyncCursorComparator,
  SyncFetch,
  SyncFetchResponse,
  SyncResponseHeaders,
  SyncPrebuiltRequest,
  SyncPrebuiltRequestTransportOptions,
  SyncRequestOptions,
  SyncRequestInit,
  SyncFetchTransportOptions,
  SyncTransportOptions,
} from './transport.js';
