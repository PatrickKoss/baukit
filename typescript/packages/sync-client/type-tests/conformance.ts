import {
  createSyncConformanceTests,
  type SyncConformanceAdapter,
  type SyncConformanceChange,
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
