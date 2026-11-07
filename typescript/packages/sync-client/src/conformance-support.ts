import type { SyncConformanceChange, SyncConformanceRow } from './conformance.js';

export interface TestContext<TClient, TServer> {
  clientA: TClient;
  clientB: TClient;
  server: TServer;
}

export const PAGE_SIZE = 2;

export const parentChange: SyncConformanceChange = {
  changeId: 'change-parent',
  entityType: 'container',
  entityId: 'parent',
  operation: 'upsert',
  value: 'parent value',
  logicalTime: 10,
};

export const childChange: SyncConformanceChange = {
  changeId: 'change-child',
  entityType: 'item',
  entityId: 'child',
  operation: 'upsert',
  value: 'child value',
  logicalTime: 11,
  dependsOn: { entityType: 'container', entityId: 'parent' },
};

export function change(
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

export function tombstone(
  changeId: string,
  entityId: string,
  logicalTime: number,
): SyncConformanceChange {
  return {
    changeId,
    entityType: 'record',
    entityId,
    operation: 'delete',
    value: null,
    logicalTime,
  };
}

export function fail(message: string): never {
  throw new Error(`Sync conformance failed: ${message}`);
}

export function assert(condition: boolean, message: string): asserts condition {
  if (!condition) fail(message);
}

export function sortedRows(rows: readonly SyncConformanceRow[]): SyncConformanceRow[] {
  return [...rows].sort(
    (left, right) =>
      left.entityType.localeCompare(right.entityType) ||
      left.entityId.localeCompare(right.entityId),
  );
}

export function sameRows(
  left: readonly SyncConformanceRow[],
  right: readonly SyncConformanceRow[],
): boolean {
  return JSON.stringify(sortedRows(left)) === JSON.stringify(sortedRows(right));
}

export async function expectFailure(run: () => Promise<void>, message: string): Promise<void> {
  try {
    await run();
  } catch {
    return;
  }
  fail(message);
}
