import { beforeEach, describe, expect, it, vi } from 'vitest';

const runtime = vi.hoisted(() => ({
  appState: 'active',
  platform: 'ios',
  appStateListener: undefined as ((state: string) => void) | undefined,
  networkListener: undefined as
    ((state: { isConnected?: boolean; isInternetReachable?: boolean }) => void) | undefined,
  removeAppState: vi.fn(),
  removeNetwork: vi.fn(),
  getNetworkStateAsync: vi.fn(),
}));

vi.mock('react-native', () => ({
  AppState: {
    get currentState() {
      return runtime.appState;
    },
    addEventListener: vi.fn(
      (_event: string, listener: (state: string) => void): { remove(): void } => {
        runtime.appStateListener = listener;
        return { remove: runtime.removeAppState };
      },
    ),
  },
  Platform: {
    get OS() {
      return runtime.platform;
    },
  },
}));

vi.mock('expo-network', () => ({
  getNetworkStateAsync: runtime.getNetworkStateAsync,
  addNetworkStateListener: vi.fn(
    (
      listener: (state: { isConnected?: boolean; isInternetReachable?: boolean }) => void,
    ): { remove(): void } => {
      runtime.networkListener = listener;
      return { remove: runtime.removeNetwork };
    },
  ),
}));

import { createExpoSyncEnvironment } from './expo.js';
import { createExpoSyncEnvironment as createFromPackageExport } from '@baukit/sync-client/expo';

describe('createExpoSyncEnvironment', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    runtime.appState = 'active';
    runtime.platform = 'ios';
    runtime.appStateListener = undefined;
    runtime.networkListener = undefined;
    runtime.getNetworkStateAsync.mockResolvedValue({
      isConnected: true,
      isInternetReachable: true,
    });
  });

  it('resolves the built Expo package export', () => {
    expect(createFromPackageExport).toBeTypeOf('function');
  });

  it('maps AppState changes onto active state and removes the subscription', () => {
    const listener = vi.fn();
    const environment = createExpoSyncEnvironment();

    expect(environment.isActive()).toBe(true);
    const unsubscribe = environment.subscribeActive(listener);
    runtime.appStateListener?.('background');
    runtime.appStateListener?.('active');

    expect(listener.mock.calls).toEqual([[false], [true]]);
    unsubscribe();
    expect(runtime.removeAppState).toHaveBeenCalledOnce();
  });

  it('reports a native transition from offline to usable connectivity once', async () => {
    runtime.getNetworkStateAsync.mockResolvedValue({
      isConnected: false,
      isInternetReachable: false,
    });
    const listener = vi.fn();
    const unsubscribe = createExpoSyncEnvironment().subscribeOnline(listener);
    await Promise.resolve();

    runtime.networkListener?.({ isConnected: true, isInternetReachable: false });
    runtime.networkListener?.({ isConnected: true, isInternetReachable: undefined });
    runtime.networkListener?.({ isConnected: true, isInternetReachable: true });

    expect(listener).toHaveBeenCalledOnce();
    unsubscribe();
    expect(runtime.removeNetwork).toHaveBeenCalledOnce();
  });

  it('uses browser online events on Expo web', () => {
    runtime.platform = 'web';
    const addEventListener = vi.fn();
    const removeEventListener = vi.fn();
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: { addEventListener, removeEventListener },
    });
    const listener = vi.fn();

    try {
      const unsubscribe = createExpoSyncEnvironment().subscribeOnline(listener);
      expect(addEventListener).toHaveBeenCalledWith('online', listener);
      unsubscribe();
      expect(removeEventListener).toHaveBeenCalledWith('online', listener);
    } finally {
      if (originalWindow) {
        Object.defineProperty(globalThis, 'window', originalWindow);
      } else {
        Reflect.deleteProperty(globalThis, 'window');
      }
    }
  });

  it('uses injected timers', () => {
    const handle = {};
    const timers = {
      setInterval: vi.fn(() => handle),
      clearInterval: vi.fn(),
    };
    const environment = createExpoSyncEnvironment({ timers });
    const callback = vi.fn();

    expect(environment.setInterval(callback, 1234)).toBe(handle);
    environment.clearInterval(handle);

    expect(timers.setInterval).toHaveBeenCalledWith(callback, 1234);
    expect(timers.clearInterval).toHaveBeenCalledWith(handle);
  });
});
