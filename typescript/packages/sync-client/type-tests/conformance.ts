import {
  createSyncConformanceTests,
  type SyncConformanceAdapter,
  type SyncConformanceChange,
  type SyncConformanceFullResyncCallbacks,
  type SyncConformanceStaleCursor,
  type SyncConformanceSubmittedBatchOutcome,
} from '@baukit/sync-client/conformance';

declare const adapter: SyncConformanceAdapter<
  object,
  object,
  object,
  object,
  object,
  object,
  object,
  number
>;

const tests = createSyncConformanceTests(adapter);
declare const outcome: SyncConformanceSubmittedBatchOutcome<object, object, object>;
declare const fullResync: SyncConformanceFullResyncCallbacks<object, object, number>;
declare const staleCursor: SyncConformanceStaleCursor<number>;
const legacyAcknowledge = adapter.outbox.markAcknowledged;
const legacyRejection = adapter.outbox.recordRejected;
const fixture: SyncConformanceChange = {
  changeId: 'change-1',
  entityType: 'record',
  entityId: 'record-1',
  operation: 'delete',
  value: null,
  logicalTime: 1,
};

void tests;
void fixture;
void outcome;
void fullResync;
void staleCursor;
void legacyAcknowledge;
void legacyRejection;
