import type { SyncSchedulerEnvironment } from '@baukit/sync-client';
import {
  createExpoSyncEnvironment,
  type ExpoSyncEnvironmentOptions,
} from '@baukit/sync-client/expo';

const options: ExpoSyncEnvironmentOptions = {};
const environment: SyncSchedulerEnvironment = createExpoSyncEnvironment(options);

export { environment, options };
