import type { SyncConformanceTestCase } from './conformance.js';
import type { SyncConformanceKit } from './conformance-kit.js';
import {
  assert,
  change,
  childChange,
  expectFailure,
  parentChange,
  sameRows,
  tombstone,
} from './conformance-support.js';

export function coreSyncConformanceCases<
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
): SyncConformanceTestCase[] {
  return [
    ...replayCases(kit),
    ...cursorCases(kit),
    ...pullValidationCases(kit),
    ...submissionOrderingCases(kit),
    ...failureCases(kit),
    ...rejectionCases(kit),
    ...convergenceCases(kit),
  ];
}

function replayCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, pending } = kit;
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
  ];
}

function cursorCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, pullPage, pullAll } = kit;
  return [
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
  ];
}

function pullValidationCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, push, pullPage, pullAll } = kit;
  return [
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
  ];
}

function submissionOrderingCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, pending, assertPendingState, submit, applySubmittedBatchOutcome } =
    kit;
  return [
    {
      name: 'an older accepted submission settles only its exact pending rows',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.outbox.enqueue(clientA, change('change-overlap-a', 'overlap', 10, 'A'));
          const submissionA = await submit(clientA, server);
          await adapter.outbox.enqueue(clientA, change('change-overlap-b', 'overlap', 30, 'B'));
          await submit(clientA, server);

          await applySubmittedBatchOutcome(clientA, submissionA);

          const remaining = await pending(clientA);
          assert(remaining.length === 1, 'accepted submission removed a newer pending row');
          assert(
            remaining[0] !== undefined &&
              adapter.outbox.pendingId(remaining[0]) === 'change-overlap-b',
            'accepted submission settled the wrong pending ID',
          );
          const rows = await adapter.local.snapshot(clientA);
          const visible = rows.find(
            (row) => row.entityType === 'record' && row.entityId === 'overlap',
          );
          assert(visible?.value === 'B', 'accepted submission replaced the newer local value');
          await assertPendingState(clientA, 1);
        }),
    },
    {
      name: 'an older rejected submission cannot replace a newer pending value',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.server.seed(server, change('seed-overlap', 'overlap', 20, 'server'));
          await adapter.outbox.enqueue(clientA, change('change-overlap-a', 'overlap', 10, 'A'));
          const submissionA = await submit(clientA, server);
          await adapter.outbox.enqueue(clientA, change('change-overlap-b', 'overlap', 30, 'B'));
          await submit(clientA, server);

          await applySubmittedBatchOutcome(clientA, submissionA);

          const remaining = await pending(clientA);
          assert(remaining.length === 1, 'rejected submission removed a newer pending row');
          assert(
            remaining[0] !== undefined &&
              adapter.outbox.pendingId(remaining[0]) === 'change-overlap-b',
            'rejected submission settled the wrong pending ID',
          );
          const rows = await adapter.local.snapshot(clientA);
          const visible = rows.find(
            (row) => row.entityType === 'record' && row.entityId === 'overlap',
          );
          assert(visible?.value === 'B', 'rejected server row replaced the newer local value');
          assert(
            (await adapter.outbox.listRejected(clientA)).length === 1,
            'rejected submission did not record its rejection',
          );
          await assertPendingState(clientA, 1);
        }),
    },
  ];
}

function failureCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, push, pullPage } = kit;
  return [
    {
      name: 'a stale pull commits its cursor without replacing a pending local value',
      run: () =>
        inScenario(async ({ clientA, server }) => {
          await adapter.server.seed(server, change('seed-stale-pull', 'stale-pull', 20, 'server'));
          await adapter.outbox.enqueue(
            clientA,
            change('change-stale-pull', 'stale-pull', 20, 'local'),
          );
          const cursorBefore = await adapter.local.readCursor(clientA);

          await pullPage(clientA, server);

          const cursorAfter = await adapter.local.readCursor(clientA);
          assert(
            adapter.wire.compareCursors(cursorAfter, cursorBefore) > 0,
            'stale pull did not commit its cursor',
          );
          const rows = await adapter.local.snapshot(clientA);
          const visible = rows.find(
            (row) => row.entityType === 'record' && row.entityId === 'stale-pull',
          );
          assert(visible?.value === 'local', 'stale pull replaced the pending local value');
          await assertPendingState(clientA, 1);
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
  ];
}

function rejectionCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, push } = kit;
  return [
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
  ];
}

function convergenceCases<
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
): SyncConformanceTestCase[] {
  const { adapter, inScenario, assertPendingState, push, pullAll } = kit;
  return [
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
