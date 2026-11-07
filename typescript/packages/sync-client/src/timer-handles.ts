import type { SyncSchedulerTimer } from './scheduler.js';

export interface HostIntervalTimers<THandle> {
  setInterval(callback: () => void, milliseconds: number): THandle;
  clearInterval(handle: THandle): void;
}

export interface OpaqueIntervalTimers {
  setInterval(callback: () => void, milliseconds: number): SyncSchedulerTimer;
  clearInterval(handle: SyncSchedulerTimer): void;
}

/** Wraps host interval ids, whatever their runtime type, in opaque object handles. */
export function createOpaqueIntervalTimers<THandle>(
  host: HostIntervalTimers<THandle>,
): OpaqueIntervalTimers {
  const hostHandles = new WeakMap<SyncSchedulerTimer, { readonly id: THandle }>();
  return {
    setInterval(callback, milliseconds) {
      const handle: SyncSchedulerTimer = {};
      hostHandles.set(handle, { id: host.setInterval(callback, milliseconds) });
      return handle;
    },
    clearInterval(handle) {
      const hostHandle = hostHandles.get(handle);
      if (hostHandle === undefined) return;
      hostHandles.delete(handle);
      host.clearInterval(hostHandle.id);
    },
  };
}
