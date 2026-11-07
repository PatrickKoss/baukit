import { SyncConformanceKit } from './conformance-kit.js';
import { coreSyncConformanceCases } from './conformance-cases.js';
import { fullResyncConformanceCases } from './conformance-resync-cases.js';

export type SyncConformanceOperation = 'upsert' | 'delete';
export type SyncConformancePushFailure = 'network' | 'server';
export type SyncConformancePullFault = 'regress' | 'stall';

export interface SyncConformanceEntityRef {
  entityType: string;
  entityId: string;
}

export interface SyncConformanceChange extends SyncConformanceEntityRef {
  changeId: string;
  operation: SyncConformanceOperation;
  value: string | null;
  logicalTime: number;
  dependsOn?: SyncConformanceEntityRef;
}

export interface SyncConformanceRow extends SyncConformanceEntityRef {
  value: string | null;
  logicalTime: number;
  deleted: boolean;
}

export interface SyncConformanceRejection extends SyncConformanceEntityRef {
  reason: string;
}

export interface SyncConformancePullPage<TChange, TCursor> {
  changes: readonly TChange[];
  nextCursor: TCursor;
  hasMore: boolean;
}

export interface SyncConformancePushResult<TPending, TRemoteChange = never> {
  acknowledged: readonly TPending[];
  rejected: readonly SyncConformanceRejection[];
  rejectedRows?: readonly TRemoteChange[];
}

export interface SyncConformanceSubmittedBatchOutcome<
  TPending,
  TPushResponse,
  TRemoteChange,
> extends SyncConformancePushResult<TPending, TRemoteChange> {
  submitted: readonly TPending[];
  response: TPushResponse;
}

export interface SyncConformancePendingState {
  pending: boolean;
  pendingCount: number;
}

export interface SyncConformanceLocalApplyOptions {
  failAfterChanges?: number;
}

export interface SyncConformanceStaleCursor<TCursor> {
  code: 'resync_required';
  horizon: TCursor;
}

export interface SyncConformanceFullResyncOptions {
  failAfterRows?: number;
}

export interface SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor> {
  zeroCursor: TCursor;
  decodeStaleCursor(error: unknown): SyncConformanceStaleCursor<TCursor> | null;
  isResetSafe(
    client: TClient,
    stale: SyncConformanceStaleCursor<TCursor>,
  ): Promise<boolean> | boolean;
  setResetSafe(client: TClient, safe: boolean): Promise<void> | void;
  reset(
    client: TClient,
    stale: SyncConformanceStaleCursor<TCursor>,
    options?: SyncConformanceFullResyncOptions,
  ): Promise<void> | void;
  readResetCount(client: TClient): Promise<number> | number;
  purgeTombstones(
    server: TServer,
    rows: readonly SyncConformanceEntityRef[],
  ): Promise<TCursor> | TCursor;
  rejectNextZeroCursor(server: TServer, horizon: TCursor): Promise<void> | void;
}

export interface SyncConformanceScenario<TClient, TServer> {
  clients: readonly [TClient, TClient];
  server: TServer;
}

export interface SyncConformanceAdapter<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
> {
  createScenario():
    Promise<SyncConformanceScenario<TClient, TServer>> | SyncConformanceScenario<TClient, TServer>;
  disposeScenario?(scenario: SyncConformanceScenario<TClient, TServer>): Promise<void> | void;
  outbox: {
    enqueue(client: TClient, change: SyncConformanceChange): Promise<void> | void;
    listPending(client: TClient): Promise<readonly TPending[]> | readonly TPending[];
    pendingId(pending: TPending): string;
    listRejected(
      client: TClient,
    ): Promise<readonly SyncConformanceRejection[]> | readonly SyncConformanceRejection[];
  };
  local: {
    readCursor(client: TClient): Promise<TCursor> | TCursor;
    applyPullPage(
      client: TClient,
      page: SyncConformancePullPage<TRemoteChange, TCursor>,
      options?: SyncConformanceLocalApplyOptions,
    ): Promise<void> | void;
    snapshot(
      client: TClient,
    ): Promise<readonly SyncConformanceRow[]> | readonly SyncConformanceRow[];
    readPendingState(
      client: TClient,
    ): Promise<SyncConformancePendingState> | SyncConformancePendingState;
    applySubmittedBatchOutcome(
      client: TClient,
      outcome: SyncConformanceSubmittedBatchOutcome<TPending, TPushResponse, TRemoteChange>,
    ): Promise<void> | void;
  };
  wire: {
    encodePush(pending: readonly TPending[]): Promise<TPushRequest> | TPushRequest;
    decodePush(
      response: TPushResponse,
      submitted: readonly TPending[],
    ):
      | Promise<SyncConformancePushResult<TPending, TRemoteChange>>
      | SyncConformancePushResult<TPending, TRemoteChange>;
    decodePull(
      response: TPullResponse,
    ):
      | Promise<SyncConformancePullPage<TRemoteChange, TCursor>>
      | SyncConformancePullPage<TRemoteChange, TCursor>;
    compareCursors(left: TCursor, right: TCursor): number;
  };
  server: {
    acceptPush(server: TServer, request: TPushRequest): Promise<TPushResponse>;
    servePull(server: TServer, cursor: TCursor, pageSize: number): Promise<TPullResponse>;
    seed(server: TServer, change: SyncConformanceChange): Promise<void> | void;
    snapshot(
      server: TServer,
    ): Promise<readonly SyncConformanceRow[]> | readonly SyncConformanceRow[];
    failNextPush(server: TServer, failure: SyncConformancePushFailure): Promise<void> | void;
    omitNextPushOutcome(server: TServer): Promise<void> | void;
    faultNextPull(server: TServer, fault: SyncConformancePullFault): Promise<void> | void;
  };
  fullResync?: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>;
}

export interface SyncConformanceTestCase {
  readonly name: string;
  readonly run: () => Promise<void>;
}

export function createSyncConformanceTests<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  adapter: SyncConformanceAdapter<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
): readonly SyncConformanceTestCase[] {
  const kit = new SyncConformanceKit(adapter);
  const tests = coreSyncConformanceCases(kit);
  const fullResync = adapter.fullResync;
  if (fullResync === undefined) return tests;
  return [...tests, ...fullResyncConformanceCases(kit, fullResync)];
}
