import { validatePullPage } from './transport.js';

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

export interface SyncConformancePushResult<TPending> {
  acknowledged: readonly TPending[];
  rejected: readonly SyncConformanceRejection[];
}

export interface SyncConformancePendingState {
  pending: boolean;
  pendingCount: number;
}

export interface SyncConformanceLocalApplyOptions {
  failAfterChanges?: number;
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
    markAcknowledged(client: TClient, pending: readonly TPending[]): Promise<void> | void;
    recordRejected(
      client: TClient,
      rejected: readonly SyncConformanceRejection[],
    ): Promise<void> | void;
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
  };
  wire: {
    encodePush(pending: readonly TPending[]): Promise<TPushRequest> | TPushRequest;
    decodePush(
      response: TPushResponse,
      submitted: readonly TPending[],
    ): Promise<SyncConformancePushResult<TPending>> | SyncConformancePushResult<TPending>;
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
}

export interface SyncConformanceTestCase {
  readonly name: string;
  readonly run: () => Promise<void>;
}

interface TestContext<TClient, TServer> {
  clientA: TClient;
  clientB: TClient;
  server: TServer;
}

const PAGE_SIZE = 2;

const parentChange: SyncConformanceChange = {
  changeId: 'change-parent',
  entityType: 'container',
  entityId: 'parent',
  operation: 'upsert',
  value: 'parent value',
  logicalTime: 10,
};

const childChange: SyncConformanceChange = {
  changeId: 'change-child',
  entityType: 'item',
  entityId: 'child',
  operation: 'upsert',
  value: 'child value',
  logicalTime: 11,
  dependsOn: { entityType: 'container', entityId: 'parent' },
};

function change(
  changeId: string,
  entityId: string,
  logicalTime: number,
  value: string,
): SyncConformanceChange {
  return {
    changeId,
    entityType: 'record',
    entityId,
    operation: 'upsert',
    value,
    logicalTime,
  };
}

function tombstone(changeId: string, entityId: string, logicalTime: number): SyncConformanceChange {
  return {
    changeId,
    entityType: 'record',
    entityId,
    operation: 'delete',
    value: null,
    logicalTime,
  };
}

function fail(message: string): never {
  throw new Error(`Sync conformance failed: ${message}`);
}

function assert(condition: boolean, message: string): asserts condition {
  if (!condition) fail(message);
}

function sortedRows(rows: readonly SyncConformanceRow[]): SyncConformanceRow[] {
  return [...rows].sort(
    (left, right) =>
      left.entityType.localeCompare(right.entityType) ||
      left.entityId.localeCompare(right.entityId),
  );
}

function sameRows(
  left: readonly SyncConformanceRow[],
  right: readonly SyncConformanceRow[],
): boolean {
  return JSON.stringify(sortedRows(left)) === JSON.stringify(sortedRows(right));
}

async function expectFailure(run: () => Promise<void>, message: string): Promise<void> {
  try {
    await run();
  } catch {
    return;
  }
  fail(message);
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
  type Scenario = SyncConformanceScenario<TClient, TServer>;

  async function inScenario(run: (context: TestContext<TClient, TServer>) => Promise<void>) {
    const scenario: Scenario = await adapter.createScenario();
    const [clientA, clientB] = scenario.clients;
    try {
      await run({ clientA, clientB, server: scenario.server });
    } finally {
      await adapter.disposeScenario?.(scenario);
    }
  }

  async function pending(client: TClient): Promise<readonly TPending[]> {
    return adapter.outbox.listPending(client);
  }

  async function assertPendingState(client: TClient, expectedCount: number): Promise<void> {
    const actual = await pending(client);
    const reported = await adapter.local.readPendingState(client);
    assert(actual.length === expectedCount, `expected ${String(expectedCount)} pending changes`);
    assert(
      reported.pendingCount === actual.length,
      'reported pending count differs from the outbox',
    );
    assert(reported.pending === actual.length > 0, 'reported pending flag differs from the outbox');
  }

  async function push(client: TClient, server: TServer): Promise<void> {
    const submitted = await pending(client);
    if (submitted.length === 0) return;
    const request = await adapter.wire.encodePush(submitted);
    const response = await adapter.server.acceptPush(server, request);
    const outcome = await adapter.wire.decodePush(response, submitted);
    await adapter.outbox.markAcknowledged(client, outcome.acknowledged);
    await adapter.outbox.recordRejected(client, outcome.rejected);
  }

  async function pullPage(
    client: TClient,
    server: TServer,
    options?: SyncConformanceLocalApplyOptions,
  ): Promise<boolean> {
    const cursor = (await adapter.local.readCursor(client)) as TCursor;
    const response = await adapter.server.servePull(server, cursor, PAGE_SIZE);
    const page = await adapter.wire.decodePull(response);
    validatePullPage(cursor, page, (left, right) => adapter.wire.compareCursors(left, right));
    await adapter.local.applyPullPage(client, page, options);
    return page.hasMore;
  }

  async function pullAll(client: TClient, server: TServer): Promise<void> {
    let hasMore = true;
    while (hasMore) hasMore = await pullPage(client, server);
  }

  return [
    {
      name: 'replayed pushes create one server change',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-replay', 'replay', 10, 'once'));
          const submitted = await pending(clientA);
          const request = await adapter.wire.encodePush(submitted);
          const first = await adapter.server.acceptPush(server, request);
          const second = await adapter.server.acceptPush(server, request);
          await adapter.wire.decodePush(first, submitted);
          await adapter.wire.decodePush(second, submitted);
          const rows = (await adapter.server.snapshot(server)).filter(
            (row) => row.entityType === 'record' && row.entityId === 'replay',
          );
          assert(rows.length === 1, 'replay created a duplicate server row');
        }),
    },
    {
      name: 'an interrupted local transaction preserves data, cursor, and pending work',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-pending', 'pending', 30, 'local'));
          await adapter.server.seed(server, change('seed-first', 'first', 10, 'remote first'));
          await adapter.server.seed(server, change('seed-second', 'second', 20, 'remote second'));
          const cursorBefore = await adapter.local.readCursor(clientA);
          const rowsBefore = await adapter.local.snapshot(clientA);
          await expectFailure(
            () => pullPage(clientA, server, { failAfterChanges: 1 }).then(() => undefined),
            'local apply failure resolved successfully',
          );
          const cursorAfter = await adapter.local.readCursor(clientA);
          const rowsAfter = await adapter.local.snapshot(clientA);
          assert(
            adapter.wire.compareCursors(cursorAfter, cursorBefore) === 0,
            'local apply failure advanced the cursor',
          );
          assert(sameRows(rowsAfter, rowsBefore), 'local apply failure committed part of the page');
          await assertPendingState(clientA, 1);
        }),
    },
    {
      name: 'pull cursors advance monotonically across pages',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.server.seed(server, change('seed-one', 'one', 10, 'one'));
          await adapter.server.seed(server, change('seed-two', 'two', 20, 'two'));
          await adapter.server.seed(server, change('seed-three', 'three', 30, 'three'));
          const start = await adapter.local.readCursor(clientA);
          await pullAll(clientA, server);
          const end = await adapter.local.readCursor(clientA);
          assert(adapter.wire.compareCursors(end, start) > 0, 'pull did not advance the cursor');
          assert(
            sameRows(await adapter.local.snapshot(clientA), await adapter.server.snapshot(server)),
            'paged pull skipped or duplicated server state',
          );
        }),
    },
    {
      name: 'a regressing pull cursor is rejected before local apply',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.server.seed(server, change('seed-regress', 'regress', 10, 'remote'));
          await pullAll(clientA, server);
          const cursor = await adapter.local.readCursor(clientA);
          const rows = await adapter.local.snapshot(clientA);
          await adapter.server.faultNextPull(server, 'regress');
          await expectFailure(
            () => pullPage(clientA, server).then(() => undefined),
            'regressing cursor was accepted',
          );
          assert(
            adapter.wire.compareCursors(await adapter.local.readCursor(clientA), cursor) === 0,
            'regressing page changed the cursor',
          );
          assert(
            sameRows(await adapter.local.snapshot(clientA), rows),
            'regressing page changed data',
          );
        }),
    },
    {
      name: 'hasMore requires cursor progress',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const cursor = await adapter.local.readCursor(clientA);
          await adapter.server.faultNextPull(server, 'stall');
          await expectFailure(
            () => pullPage(clientA, server).then(() => undefined),
            'hasMore without cursor progress was accepted',
          );
          assert(
            adapter.wire.compareCursors(await adapter.local.readCursor(clientA), cursor) === 0,
            'stalled page changed the cursor',
          );
        }),
    },
    {
      name: 'partial push outcomes are rejected before acknowledgement',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-covered-a', 'covered-a', 10, 'a'));
          await adapter.outbox.enqueue(clientA, change('change-covered-b', 'covered-b', 20, 'b'));
          await adapter.server.omitNextPushOutcome(server);
          await expectFailure(() => push(clientA, server), 'partial push outcomes were accepted');
          await assertPendingState(clientA, 2);
        }),
    },
    {
      name: 'network failure keeps pending state truthful',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-network', 'network', 10, 'local'));
          await adapter.server.failNextPush(server, 'network');
          await expectFailure(() => push(clientA, server), 'network failure resolved successfully');
          await assertPendingState(clientA, 1);
        }),
    },
    {
      name: 'server failure keeps pending state truthful',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-server', 'server', 10, 'local'));
          await adapter.server.failNextPush(server, 'server');
          await expectFailure(() => push(clientA, server), 'server failure resolved successfully');
          await assertPendingState(clientA, 1);
        }),
    },
    {
      name: 'complete rejection outcomes are recorded without hidden pending work',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.server.seed(server, change('seed-newer', 'rejected', 20, 'server'));
          await adapter.outbox.enqueue(clientA, change('change-older', 'rejected', 10, 'local'));
          await push(clientA, server);
          await assertPendingState(clientA, 0);
          const rejected = await adapter.outbox.listRejected(clientA);
          assert(rejected.length === 1, 'actionable rejection was not recorded');
          assert(rejected[0]?.entityId === 'rejected', 'wrong rejection was recorded');
        }),
    },
    {
      name: 'pushes referenced records before dependent records',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, childChange);
          await adapter.outbox.enqueue(clientA, parentChange);
          await push(clientA, server);
          await assertPendingState(clientA, 0);
          const rows = await adapter.server.snapshot(server);
          assert(
            rows.some((row) => row.entityType === 'container' && row.entityId === 'parent'),
            'referenced record did not reach the server',
          );
          assert(
            rows.some((row) => row.entityType === 'item' && row.entityId === 'child'),
            'dependent record was rejected because push order was unsafe',
          );
        }),
    },
    {
      name: 'two clients converge after alternating sync, including a tombstone',
      run: () =>
        inScenario(async ({ clientA, clientB, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-a', 'alpha', 10, 'from A'));
          await adapter.outbox.enqueue(clientB, change('change-b', 'beta', 20, 'from B'));
          await push(clientA, server);
          await pullAll(clientB, server);
          await push(clientB, server);
          await pullAll(clientA, server);
          await adapter.outbox.enqueue(clientB, tombstone('change-delete', 'alpha', 30));
          await push(clientB, server);
          await pullAll(clientA, server);
          await pullAll(clientB, server);

          const rowsA = await adapter.local.snapshot(clientA);
          const rowsB = await adapter.local.snapshot(clientB);
          const serverRows = await adapter.server.snapshot(server);
          assert(sameRows(rowsA, rowsB), 'clients did not converge to the same state');
          assert(sameRows(rowsA, serverRows), 'clients did not converge to server state');
          const deleted = rowsA.find(
            (row) => row.entityType === 'record' && row.entityId === 'alpha',
          );
          assert(deleted?.deleted === true, 'tombstone did not converge');
          await assertPendingState(clientA, 0);
          await assertPendingState(clientB, 0);
        }),
    },
  ];
}
