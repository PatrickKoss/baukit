import { describe, expect, it, vi } from 'vitest';

import {
  createBrowserSyncEnvironment,
  type BrowserSyncDocument,
  type BrowserSyncWindow,
} from '@baukit/sync-client/browser';

interface MutableBrowserSyncDocument extends BrowserSyncDocument {
  visibilityState: string;
  dispatchVisibilityChange(): void;
  readonly removeVisibility: ReturnType<typeof vi.fn>;
}

interface TestBrowserSyncWindow extends BrowserSyncWindow {
  dispatchOnline(): void;
  readonly removeOnline: ReturnType<typeof vi.fn>;
}

function documentSource(initialState: string): MutableBrowserSyncDocument {
  const listeners = new Set<() => void>();
  const removeVisibility = vi.fn((_type: 'visibilitychange', listener: () => void) => {
    listeners.delete(listener);
  });
  return {
    visibilityState: initialState,
    addEventListener: (_type, listener) => {
      listeners.add(listener);
    },
    removeEventListener: removeVisibility,
    dispatchVisibilityChange: () => {
      listeners.forEach((listener) => {
        listener();
      });
    },
    removeVisibility,
  };
}

function windowSource(): TestBrowserSyncWindow {
  const listeners = new Set<() => void>();
  const removeOnline = vi.fn((_type: 'online', listener: () => void) => {
    listeners.delete(listener);
  });
  return {
    addEventListener: (_type, listener) => {
      listeners.add(listener);
    },
    removeEventListener: removeOnline,
    setInterval: vi.fn(() => 1),
    clearInterval: vi.fn(),
    dispatchOnline: () => {
      listeners.forEach((listener) => {
        listener();
      });
    },
    removeOnline,
  };
}

describe('createBrowserSyncEnvironment', () => {
  it.each([
    ['hidden', false],
    ['visible', true],
  ])('reports %s startup state', (visibilityState, expected) => {
    const environment = createBrowserSyncEnvironment({
      document: documentSource(visibilityState),
      window: windowSource(),
    });

    expect(environment.isActive()).toBe(expected);
  });

  it('publishes hidden and visible transitions', () => {
    const document = documentSource('visible');
    const active = vi.fn();
    const environment = createBrowserSyncEnvironment({ document, window: windowSource() });
    environment.subscribeActive(active);

    document.visibilityState = 'hidden';
    document.dispatchVisibilityChange();
    document.visibilityState = 'visible';
    document.dispatchVisibilityChange();

    expect(active.mock.calls).toEqual([[false], [true]]);
  });

  it('publishes browser online events', () => {
    const window = windowSource();
    const online = vi.fn();
    const environment = createBrowserSyncEnvironment({
      document: documentSource('visible'),
      window,
    });
    environment.subscribeOnline(online);

    window.dispatchOnline();

    expect(online).toHaveBeenCalledOnce();
  });

  it('delegates timers to injected functions', () => {
    const handle = {};
    const timers = {
      setInterval: vi.fn(() => handle),
      clearInterval: vi.fn(),
    };
    const environment = createBrowserSyncEnvironment({
      document: documentSource('visible'),
      window: windowSource(),
      timers,
    });
    const callback = vi.fn();

    expect(environment.setInterval(callback, 1234)).toBe(handle);
    environment.clearInterval(handle);

    expect(timers.setInterval).toHaveBeenCalledWith(callback, 1234);
    expect(timers.clearInterval).toHaveBeenCalledWith(handle);
  });

  it('removes both subscriptions once when cleanup repeats', () => {
    const document = documentSource('visible');
    const window = windowSource();
    const environment = createBrowserSyncEnvironment({
      document,
      window,
    });
    const active = vi.fn();
    const online = vi.fn();
    const unsubscribeActive = environment.subscribeActive(active);
    const unsubscribeOnline = environment.subscribeOnline(online);

    unsubscribeActive();
    unsubscribeActive();
    unsubscribeOnline();
    unsubscribeOnline();
    document.dispatchVisibilityChange();
    window.dispatchOnline();

    expect(document.removeVisibility).toHaveBeenCalledOnce();
    expect(window.removeOnline).toHaveBeenCalledOnce();
    expect(active).not.toHaveBeenCalled();
    expect(online).not.toHaveBeenCalled();
  });

  it('uses browser globals when they are present', () => {
    const originalDocument = Object.getOwnPropertyDescriptor(globalThis, 'document');
    const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
    const document = documentSource('visible');
    const window = windowSource();
    Object.defineProperty(globalThis, 'document', { configurable: true, value: document });
    Object.defineProperty(globalThis, 'window', { configurable: true, value: window });

    try {
      expect(createBrowserSyncEnvironment().isActive()).toBe(true);
    } finally {
      if (originalDocument) Object.defineProperty(globalThis, 'document', originalDocument);
      else Reflect.deleteProperty(globalThis, 'document');
      if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
      else Reflect.deleteProperty(globalThis, 'window');
    }
  });

  it('throws a specific error when browser globals are absent', () => {
    expect(() => createBrowserSyncEnvironment()).toThrow(
      'Browser sync environment requires a document',
    );
    expect(() => createBrowserSyncEnvironment({ document: documentSource('visible') })).toThrow(
      'Browser sync environment requires a window',
    );
  });
});
