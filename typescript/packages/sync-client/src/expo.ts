import * as Network from 'expo-network';
import { AppState, Platform } from 'react-native';

import type { SyncSchedulerEnvironment, SyncSchedulerTimer } from './scheduler.js';

export interface ExpoSyncTimers {
  setInterval(callback: () => void, milliseconds: number): SyncSchedulerTimer;
  clearInterval(handle: SyncSchedulerTimer): void;
}

export interface ExpoSyncEnvironmentOptions {
  /** Overrides global timers, primarily for deterministic scheduler tests. */
  timers?: ExpoSyncTimers;
}

interface ExpoNetworkModule {
  getNetworkStateAsync(): Promise<Network.NetworkState>;
  addNetworkStateListener?: (listener: (state: Network.NetworkState) => void) => { remove(): void };
}

const defaultTimers: ExpoSyncTimers = {
  setInterval: (callback, milliseconds) =>
    globalThis.setInterval(callback, milliseconds) as unknown as SyncSchedulerTimer,
  clearInterval: (handle) => {
    globalThis.clearInterval(handle as unknown as ReturnType<typeof globalThis.setInterval>);
  },
};

function isUsableNetwork(state: Network.NetworkState): boolean {
  return state.isConnected === true && state.isInternetReachable !== false;
}

function subscribeBrowserOnline(listener: () => void): () => void {
  if (typeof window === 'undefined') {
    return () => undefined;
  }
  window.addEventListener('online', listener);
  return () => {
    window.removeEventListener('online', listener);
  };
}

function subscribeNativeOnline(listener: () => void): () => void {
  const network = Network as ExpoNetworkModule;
  let subscribed = true;
  let receivedEvent = false;
  let wasUsable: boolean | null = null;

  void network
    .getNetworkStateAsync()
    .then((state) => {
      if (subscribed && !receivedEvent) {
        wasUsable = isUsableNetwork(state);
      }
    })
    .catch(() => undefined);

  const subscription = network.addNetworkStateListener?.((state) => {
    if (!subscribed) return;
    receivedEvent = true;
    const usable = isUsableNetwork(state);
    if (wasUsable === false && usable) {
      listener();
    }
    wasUsable = usable;
  });

  return () => {
    subscribed = false;
    subscription?.remove();
  };
}

/** Creates the Expo runtime environment consumed by {@link SyncScheduler}. */
export function createExpoSyncEnvironment(
  options: ExpoSyncEnvironmentOptions = {},
): SyncSchedulerEnvironment {
  const timers = options.timers ?? defaultTimers;
  return {
    isActive: () => AppState.currentState === 'active',
    subscribeActive(listener) {
      const subscription = AppState.addEventListener('change', (state) => {
        listener(state === 'active');
      });
      return () => {
        subscription.remove();
      };
    },
    subscribeOnline(listener) {
      return Platform.OS === 'web'
        ? subscribeBrowserOnline(listener)
        : subscribeNativeOnline(listener);
    },
    setInterval(callback, milliseconds) {
      return timers.setInterval(callback, milliseconds);
    },
    clearInterval(handle) {
      timers.clearInterval(handle);
    },
  };
}
