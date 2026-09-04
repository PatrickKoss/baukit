import type { SyncSchedulerEnvironment, SyncSchedulerRecoverySignal } from '@baukit/sync-client';
import {
  createBrowserSyncEnvironment,
  type BrowserSyncDocument,
  type BrowserSyncEnvironmentOptions,
  type BrowserSyncTimers,
  type BrowserSyncWindow,
} from '@baukit/sync-client/browser';

declare const injectedDocument: BrowserSyncDocument;
declare const injectedWindow: BrowserSyncWindow;
declare const timers: BrowserSyncTimers;

const options: BrowserSyncEnvironmentOptions = {
  document: injectedDocument,
  window: injectedWindow,
  timers,
};
const environment: SyncSchedulerEnvironment = createBrowserSyncEnvironment(options);
const globalEnvironment = createBrowserSyncEnvironment({ document, window });
const recoverySignal: SyncSchedulerRecoverySignal = 'online';

export { environment, globalEnvironment, options, recoverySignal };
