import type {
  SyncConformanceFullResyncCallbacks,
  SyncConformanceRow,
  SyncConformanceTestCase,
} from './conformance.js';
import type { SyncConformanceKit } from './conformance-kit.js';
import {
  PAGE_SIZE,
  assert,
  change,
  childChange,
  expectFailure,
  parentChange,
  sameRows,
} from './conformance-support.js';
import { validatePullPage } from './transport.js';

export function fullResyncConformanceCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  return [
    ...purgeHorizonCases(kit, fullResync),
    ...fullResyncResetCases(kit, fullResync),
    ...interruptedResetCases(kit, fullResync),
    ...deferredResetCases(kit, fullResync),
    ...repeatedStaleCursorCases(kit, fullResync),
  ];
}

function purgeHorizonCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  const { adapter, inScenario, pullAll, expectStalePull } = kit;
  return [
    {
      name: 'purge horizons are monotonic and cursor zero requests a full rebuild',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const first = change('horizon-first', 'horizon-first', 10, 'first');
          const second = change('horizon-second', 'horizon-second', 20, 'second');
          await adapter.server.seed(server, first);
          await adapter.server.seed(server, second);
          await pullAll(clientA, server);
          const oldCursor = await adapter.local.readCursor(clientA);

          const firstHorizon = await fullResync.purgeTombstones(server, [first]);
          const secondHorizon = await fullResync.purgeTombstones(server, [second]);
          assert(
            adapter.wire.compareCursors(firstHorizon, oldCursor) > 0,
            'first purge did not advance the owner horizon past the old cursor',
          );
          assert(
            adapter.wire.compareCursors(secondHorizon, firstHorizon) >= 0,
            'owner purge horizon regressed',
          );

          const stale = await expectStalePull(clientA, server, fullResync);
          assert(
            adapter.wire.compareCursors(stale.horizon, secondHorizon) === 0,
            'stale cursor response did not contain the current owner horizon',
          );

          const response = await adapter.server.servePull(server, fullResync.zeroCursor, PAGE_SIZE);
          const page = await adapter.wire.decodePull(response);
          validatePullPage(fullResync.zeroCursor, page, (left, right) =>
            adapter.wire.compareCursors(left, right),
          );
        }),
    },
  ];
}

function fullResyncResetCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, push, pullAll, expectStalePull } = kit;
  return [
    {
      name: 'full resync clears parent, child, and pull-only rows but preserves repair work',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const pullOnly = change('pull-only-seed', 'pull-only', 12, 'server only');
          const pendingRemote = change('pending-seed', 'pending-edit', 20, 'server value');
          const rejectedRemote = change('rejected-seed', 'rejected-edit', 50, 'server value');
          await adapter.server.seed(server, parentChange);
          await adapter.server.seed(server, childChange);
          await adapter.server.seed(server, pullOnly);
          await adapter.server.seed(server, pendingRemote);
          await adapter.server.seed(server, rejectedRemote);
          await pullAll(clientA, server);

          await adapter.outbox.enqueue(
            clientA,
            change('rejected-local', 'rejected-edit', 40, 'rejected value'),
          );
          await push(clientA, server);
          await adapter.outbox.enqueue(
            clientA,
            change('pending-local', 'pending-edit', 60, 'pending value'),
          );
          await fullResync.purgeTombstones(server, [childChange, parentChange, pullOnly]);
          const stale = await expectStalePull(clientA, server, fullResync);

          await fullResync.reset(clientA, stale);

          assert(
            adapter.wire.compareCursors(
              await adapter.local.readCursor(clientA),
              fullResync.zeroCursor,
            ) === 0,
            'reset did not atomically store cursor zero',
          );
          await assertPendingState(clientA, 1);
          assert(
            (await adapter.outbox.listRejected(clientA)).length === 1,
            'reset removed an explicit rejection record',
          );
          assertResetRows(await adapter.local.snapshot(clientA));

          await push(clientA, server);
          await pullAll(clientA, server);
          await assertPendingState(clientA, 0);
          assert(
            (await adapter.outbox.listRejected(clientA)).length === 1,
            'full rebuild removed an unresolved rejection',
          );
        }),
    },
  ];
}

function interruptedResetCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, pullAll, expectStalePull } = kit;
  return [
    {
      name: 'an interrupted full-resync reset rolls back rows and cursor',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const first = change('reset-first', 'reset-first', 10, 'first');
          const second = change('reset-second', 'reset-second', 20, 'second');
          await adapter.server.seed(server, first);
          await adapter.server.seed(server, second);
          await pullAll(clientA, server);
          await adapter.outbox.enqueue(clientA, change('reset-pending', 'pending', 30, 'local'));
          await fullResync.purgeTombstones(server, [first, second]);
          const stale = await expectStalePull(clientA, server, fullResync);
          const cursorBefore = await adapter.local.readCursor(clientA);
          const rowsBefore = await adapter.local.snapshot(clientA);

          await expectFailure(
            () => Promise.resolve(fullResync.reset(clientA, stale, { failAfterRows: 1 })),
            'injected full-resync reset failure resolved successfully',
          );

          assert(
            adapter.wire.compareCursors(await adapter.local.readCursor(clientA), cursorBefore) ===
              0,
            'failed full-resync reset changed the cursor',
          );
          assert(
            sameRows(await adapter.local.snapshot(clientA), rowsBefore),
            'failed full-resync reset committed a partial row deletion',
          );
          await assertPendingState(clientA, 1);
        }),
    },
  ];
}

function deferredResetCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  const { adapter, inScenario, pullAll, recoverFromStaleCursor } = kit;
  return [
    {
      name: 'the product safety hook can defer a disruptive reset',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const remote = change('defer-seed', 'defer', 10, 'remote');
          await adapter.server.seed(server, remote);
          await pullAll(clientA, server);
          await fullResync.purgeTombstones(server, [remote]);
          await fullResync.setResetSafe(clientA, false);
          const cursorBefore = await adapter.local.readCursor(clientA);
          const rowsBefore = await adapter.local.snapshot(clientA);

          const result = await recoverFromStaleCursor(clientA, server, fullResync);

          assert(result === 'deferred', 'unsafe reset was not deferred');
          assert((await fullResync.readResetCount(clientA)) === 0, 'deferred reset ran anyway');
          assert(
            adapter.wire.compareCursors(await adapter.local.readCursor(clientA), cursorBefore) ===
              0,
            'deferred reset changed the cursor',
          );
          assert(
            sameRows(await adapter.local.snapshot(clientA), rowsBefore),
            'deferred reset changed local rows',
          );
        }),
    },
  ];
}

function repeatedStaleCursorCases<
  TClient,
  TServer,
  TPending,
  TPushRequest,
  TPushResponse,
  TPullResponse,
  TRemoteChange,
  TCursor,
>(
  kit: SyncConformanceKit<
    TClient,
    TServer,
    TPending,
    TPushRequest,
    TPushResponse,
    TPullResponse,
    TRemoteChange,
    TCursor
  >,
  fullResync: SyncConformanceFullResyncCallbacks<TClient, TServer, TCursor>,
): SyncConformanceTestCase[] {
  const { adapter, inScenario, pullAll, recoverFromStaleCursor } = kit;
  return [
    {
      name: 'a second stale cursor after reset stops without another reset',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          const remote = change('repeat-stale-seed', 'repeat-stale', 10, 'remote');
          await adapter.server.seed(server, remote);
          await pullAll(clientA, server);
          const horizon = await fullResync.purgeTombstones(server, [remote]);
          await fullResync.rejectNextZeroCursor(server, horizon);

          await expectFailure(
            () => recoverFromStaleCursor(clientA, server, fullResync).then(() => undefined),
            'second stale cursor restarted full resync',
          );

          assert(
            (await fullResync.readResetCount(clientA)) === 1,
            'second stale cursor caused another reset',
          );
          assert(
            adapter.wire.compareCursors(
              await adapter.local.readCursor(clientA),
              fullResync.zeroCursor,
            ) === 0,
            'failed full rebuild did not leave the explicit rebuild cursor',
          );
        }),
    },
  ];
}

function assertResetRows(resetRows: readonly SyncConformanceRow[]): void {
  assert(
    !resetRows.some((row) => row.entityType === 'container' && row.entityId === 'parent'),
    'reset retained a server-backed parent',
  );
  assert(
    !resetRows.some((row) => row.entityType === 'item' && row.entityId === 'child'),
    'reset retained a server-backed child',
  );
  assert(
    !resetRows.some((row) => row.entityType === 'record' && row.entityId === 'pull-only'),
    'reset retained a pull-only row',
  );
  assert(
    resetRows.some((row) => row.entityId === 'pending-edit' && row.value === 'pending value'),
    'reset removed the row for a pending local edit',
  );
  assert(
    resetRows.some((row) => row.entityId === 'rejected-edit'),
    'reset removed the row attached to an explicit rejection',
  );
}
