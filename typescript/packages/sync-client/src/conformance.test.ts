import { describe, expect, it } from 'vitest';
import { createSyncConformanceTests } from '@baukit/sync-client/conformance';

import {
  type SyncConformanceAdapter,
  type SyncConformanceChange,
  type SyncConformancePullFault,
  type SyncConformancePullPage,
  type SyncConformancePushFailure,
  type SyncConformanceRejection,
  type SyncConformanceRow,
} from './conformance.js';
import { SyncNetworkError, SyncServerError } from './error.js';
import { validatePushOutcomeCoverage } from './push-batch.js';

interface FakeClient {
  cursor: number;
  outbox: SyncConformanceChange[];
  rejected: SyncConformanceRejection[];
  rows: Map<string, SyncConformanceRow>;
}

interface FakeServerRow {
  row: SyncConformanceRow;
  revision: number;
}

interface FakeServer {
  appliedChangeIds: Set<string>;
  nextPullFault: SyncConformancePullFault | null;
  nextPushFailure: SyncConformancePushFailure | null;
  omitOutcome: boolean;
  revision: number;
  rows: Map<string, FakeServerRow>;
}

interface FakePushRequest {
  changes: readonly SyncConformanceChange[];
}

interface FakePushOutcome {
  entityType: string;
  entityId: string;
  status: 'accepted' | 'rejected';
  reason?: string;
  serverRow?: SyncConformanceRow;
}

interface FakePushResponse {
  outcomes: readonly FakePushOutcome[];
}

type FakePullResponse = SyncConformancePullPage<SyncConformanceRow, number>;

function key(value: { entityType: string; entityId: string }): string {
  return `${value.entityType}:${value.entityId}`;
}

function clone<T>(value: T): T {
  return structuredClone(value);
}

function createClient(): FakeClient {
  return { cursor: 0, outbox: [], rejected: [], rows: new Map() };
}

function createServer(): FakeServer {
  return {
    appliedChangeIds: new Set(),
    nextPullFault: null,
    nextPushFailure: null,
    omitOutcome: false,
    revision: 0,
    rows: new Map(),
  };
}

function rowFromChange(change: SyncConformanceChange): SyncConformanceRow {
  return {
    entityType: change.entityType,
    entityId: change.entityId,
    value: change.value,
    logicalTime: change.logicalTime,
    deleted: change.operation === 'delete',
  };
}

function applyLocalChange(rows: Map<string, SyncConformanceRow>, row: SyncConformanceRow): void {
  const existing = rows.get(key(row));
  if (!existing || row.logicalTime >= existing.logicalTime) {
    rows.set(key(row), clone(row));
  }
}

function dependencyOrder(changes: readonly SyncConformanceChange[]): SyncConformanceChange[] {
  const remaining = [...changes];
  const ordered: SyncConformanceChange[] = [];
  const available = new Set<string>();
  while (remaining.length > 0) {
    const index = remaining.findIndex(
      (candidate) => candidate.dependsOn === undefined || available.has(key(candidate.dependsOn)),
    );
    const next = remaining.splice(index < 0 ? 0 : index, 1)[0];
    if (next === undefined) throw new Error('Expected a pending change');
    ordered.push(next);
    available.add(key(next));
  }
  return ordered;
}

function applyServerChange(server: FakeServer, change: SyncConformanceChange): FakePushOutcome {
  if (server.appliedChangeIds.has(change.changeId)) {
    return { entityType: change.entityType, entityId: change.entityId, status: 'accepted' };
  }
  const existing = server.rows.get(key(change));
  if (existing && change.logicalTime <= existing.row.logicalTime) {
    return {
      entityType: change.entityType,
      entityId: change.entityId,
      status: 'rejected',
      reason: 'superseded',
      serverRow: clone(existing.row),
    };
  }
  if (change.dependsOn !== undefined && !server.rows.has(key(change.dependsOn))) {
    return {
      entityType: change.entityType,
      entityId: change.entityId,
      status: 'rejected',
      reason: 'missing_reference',
    };
  }
  server.revision += 1;
  server.appliedChangeIds.add(change.changeId);
  server.rows.set(key(change), { row: rowFromChange(change), revision: server.revision });
  return { entityType: change.entityType, entityId: change.entityId, status: 'accepted' };
}

const adapter: SyncConformanceAdapter<
  FakeClient,
  FakeServer,
  SyncConformanceChange,
  FakePushRequest,
  FakePushResponse,
  FakePullResponse,
  SyncConformanceRow,
  number
> = {
  createScenario: () => ({ clients: [createClient(), createClient()], server: createServer() }),
  outbox: {
    enqueue(client, change) {
      client.outbox.push(clone(change));
      applyLocalChange(client.rows, rowFromChange(change));
    },
    listPending: (client) => clone(client.outbox),
    pendingId: (pending) => pending.changeId,
    markAcknowledged(client, pending) {
      const acknowledged = new Set(pending.map(({ changeId }) => changeId));
      client.outbox = client.outbox.filter(({ changeId }) => !acknowledged.has(changeId));
    },
    recordRejected(client, rejected) {
      client.rejected.push(...clone(rejected));
    },
    listRejected: (client) => clone(client.rejected),
  },
  local: {
    readCursor: (client) => client.cursor,
    applyPullPage(client, page, options) {
      const staged = new Map([...client.rows].map(([rowKey, row]) => [rowKey, clone(row)]));
      let applied = 0;
      for (const row of page.changes) {
        const hasPending = client.outbox.some(
          (pending) => pending.entityType === row.entityType && pending.entityId === row.entityId,
        );
        if (!hasPending) applyLocalChange(staged, row);
        applied += 1;
        if (applied === options?.failAfterChanges) {
          throw new Error('injected local transaction failure');
        }
      }
      client.rows = staged;
      client.cursor = page.nextCursor;
    },
    snapshot: (client) => clone([...client.rows.values()]),
    readPendingState: (client) => ({
      pending: client.outbox.length > 0,
      pendingCount: client.outbox.length,
    }),
    applySubmittedBatchOutcome(client, outcome) {
      const acknowledged = new Set(outcome.acknowledged.map(({ changeId }) => changeId));
      const outbox = client.outbox.filter(({ changeId }) => !acknowledged.has(changeId));
      const rejected = [...client.rejected, ...clone(outcome.rejected)];
      const rows = new Map([...client.rows].map(([rowKey, row]) => [rowKey, clone(row)]));
      for (const row of outcome.rejectedRows ?? []) {
        const hasPending = outbox.some(
          (pending) => pending.entityType === row.entityType && pending.entityId === row.entityId,
        );
        if (!hasPending) applyLocalChange(rows, row);
      }
      client.outbox = outbox;
      client.rejected = rejected;
      client.rows = rows;
    },
  },
  wire: {
    encodePush: (pending) => ({ changes: dependencyOrder(pending) }),
    decodePush(response, submitted) {
      validatePushOutcomeCoverage(submitted, response.outcomes, {
        submittedKey: key,
        outcomeKey: key,
      });
      return {
        acknowledged: submitted,
        rejected: response.outcomes
          .filter((outcome) => outcome.status === 'rejected')
          .map((outcome) => ({
            entityType: outcome.entityType,
            entityId: outcome.entityId,
            reason: outcome.reason ?? 'rejected',
          })),
        rejectedRows: response.outcomes.flatMap((outcome) =>
          outcome.status === 'rejected' && outcome.serverRow !== undefined
            ? [clone(outcome.serverRow)]
            : [],
        ),
      };
    },
    decodePull: (response) => clone(response),
    compareCursors: (left, right) => left - right,
  },
  server: {
    acceptPush(server, request) {
      const failure = server.nextPushFailure;
      server.nextPushFailure = null;
      if (failure === 'network') throw new SyncNetworkError('offline');
      if (failure === 'server') throw new SyncServerError('unavailable', true);
      const outcomes = request.changes.map((change) => applyServerChange(server, change));
      if (server.omitOutcome) {
        server.omitOutcome = false;
        outcomes.pop();
      }
      return Promise.resolve({ outcomes: clone(outcomes) });
    },
    servePull(server, cursor, pageSize) {
      const fault = server.nextPullFault;
      server.nextPullFault = null;
      if (fault === 'stall') {
        return Promise.resolve({ changes: [], nextCursor: cursor, hasMore: true });
      }
      if (fault === 'regress') {
        return Promise.resolve({ changes: [], nextCursor: cursor - 1, hasMore: false });
      }
      const available = [...server.rows.values()]
        .filter(({ revision }) => revision > cursor)
        .sort((left, right) => left.revision - right.revision);
      const selected = available.slice(0, pageSize);
      return Promise.resolve({
        changes: clone(selected.map(({ row }) => row)),
        nextCursor: selected.at(-1)?.revision ?? cursor,
        hasMore: available.length > selected.length,
      });
    },
    seed(server, change) {
      const outcome = applyServerChange(server, change);
      if (outcome.status === 'rejected') throw new Error(outcome.reason);
    },
    snapshot: (server) => clone([...server.rows.values()].map(({ row }) => row)),
    failNextPush(server, failure) {
      server.nextPushFailure = failure;
    },
    omitNextPushOutcome(server) {
      server.omitOutcome = true;
    },
    faultNextPull(server, fault) {
      server.nextPullFault = fault;
    },
  },
};

describe('sync conformance harness', () => {
  for (const testCase of createSyncConformanceTests(adapter)) {
    it(testCase.name, () => testCase.run());
  }

  it('keeps the legacy settlement callbacks available during migration', async () => {
    const scenario = await adapter.createScenario();
    const [client] = scenario.clients;
    const pending: SyncConformanceChange = {
      changeId: 'legacy-change',
      entityType: 'record',
      entityId: 'legacy',
      operation: 'upsert',
      value: 'legacy',
      logicalTime: 1,
    };
    await adapter.outbox.enqueue(client, pending);
    await adapter.outbox.markAcknowledged(client, [pending]);
    await adapter.outbox.recordRejected(client, [
      { entityType: 'record', entityId: 'legacy', reason: 'legacy' },
    ]);

    expect(await adapter.outbox.listPending(client)).toEqual([]);
    expect(await adapter.outbox.listRejected(client)).toHaveLength(1);
  });
});
