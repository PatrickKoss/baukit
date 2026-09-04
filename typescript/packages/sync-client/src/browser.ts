import type { SyncSchedulerEnvironment, SyncSchedulerTimer } from './scheduler.js';

export interface BrowserSyncDocument {
  readonly visibilityState: string;
  addEventListener(type: 'visibilitychange', listener: () => void): void;
  removeEventListener(type: 'visibilitychange', listener: () => void): void;
}

export interface BrowserSyncWindow {
  addEventListener(type: 'online', listener: () => void): void;
  removeEventListener(type: 'online', listener: () => void): void;
  setInterval(callback: () => void, milliseconds: number): number;
  clearInterval(handle: number): void;
}

export interface BrowserSyncTimers {
  setInterval(callback: () => void, milliseconds: number): SyncSchedulerTimer;
  clearInterval(handle: SyncSchedulerTimer): void;
}

export interface BrowserSyncEnvironmentOptions {
  document?: BrowserSyncDocument;
  window?: BrowserSyncWindow;
  timers?: BrowserSyncTimers;
}

function resolveDocument(document: BrowserSyncDocument | undefined): BrowserSyncDocument {
  const globalDocument = globalThis.document as BrowserSyncDocument | undefined;
  const resolved = document ?? globalDocument;
  if (resolved === undefined) {
    throw new Error('Browser sync environment requires a document');
  }
  return resolved;
}

function resolveWindow(window: BrowserSyncWindow | undefined): BrowserSyncWindow {
  const globalWindow = globalThis.window as unknown as BrowserSyncWindow | undefined;
  const resolved = window ?? globalWindow;
  if (resolved === undefined) {
    throw new Error('Browser sync environment requires a window');
  }
  return resolved;
}

function once(cleanup: () => void): () => void {
  let active = true;
  return () => {
    if (!active) return;
    active = false;
    cleanup();
  };
}

export function createBrowserSyncEnvironment(
  options: BrowserSyncEnvironmentOptions = {},
): SyncSchedulerEnvironment {
  const browserDocument = resolveDocument(options.document);
  const browserWindow = resolveWindow(options.window);
  const timers: BrowserSyncTimers = options.timers ?? {
    setInterval: (callback, milliseconds) =>
      browserWindow.setInterval(callback, milliseconds) as unknown as SyncSchedulerTimer,
    clearInterval: (handle) => {
      browserWindow.clearInterval(handle as unknown as number);
    },
  };

  return {
    isActive: () => browserDocument.visibilityState === 'visible',
    subscribeActive(listener) {
      const onVisibilityChange = () => {
        listener(browserDocument.visibilityState === 'visible');
      };
      browserDocument.addEventListener('visibilitychange', onVisibilityChange);
      return once(() => {
        browserDocument.removeEventListener('visibilitychange', onVisibilityChange);
      });
    },
    subscribeOnline(listener) {
      browserWindow.addEventListener('online', listener);
      return once(() => {
        browserWindow.removeEventListener('online', listener);
      });
    },
    setInterval(callback, milliseconds) {
      return timers.setInterval(callback, milliseconds);
    },
    clearInterval(handle) {
      timers.clearInterval(handle);
    },
  };
}
