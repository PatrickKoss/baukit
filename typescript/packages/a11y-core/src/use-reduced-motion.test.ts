// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'web' as string };
const isReduceMotionEnabled = vi.fn<() => Promise<boolean>>();
const addEventListener =
  vi.fn<(event: string, listener: (enabled: boolean) => void) => { remove: () => void }>();

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
  AccessibilityInfo: {
    isReduceMotionEnabled: () => isReduceMotionEnabled(),
    addEventListener: (event: string, listener: (enabled: boolean) => void) =>
      addEventListener(event, listener),
  },
}));

import { useReducedMotion, useReducedMotionPreference } from './use-reduced-motion.js';

/** jsdom has no matchMedia, so the query it would return is supplied here. */
type MediaListener = (event: MediaQueryListEvent) => void;

function stubMatchMedia(matches: boolean) {
  const listeners = new Set<MediaListener>();
  const removeEventListener = vi.fn((_event: string, listener: MediaListener) => {
    listeners.delete(listener);
  });
  const matchMedia = vi.fn((query: string) => ({
    matches,
    media: query,
    addEventListener: (_event: string, listener: MediaListener) => {
      listeners.add(listener);
    },
    removeEventListener,
  }));
  vi.stubGlobal('matchMedia', matchMedia);
  return {
    matchMedia,
    removeEventListener,
    change(next: boolean) {
      for (const listener of listeners) listener({ matches: next } as MediaQueryListEvent);
    },
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  isReduceMotionEnabled.mockReset();
  addEventListener.mockReset();
  platform.OS = 'web';
});

describe('useReducedMotion on web', () => {
  it('reads the media query and resolves on the first render', () => {
    stubMatchMedia(true);

    expect(renderHook(() => useReducedMotion()).result.current).toBe(true);
    expect(renderHook(() => useReducedMotionPreference()).result.current).toEqual({
      reducedMotion: true,
      resolved: true,
    });
  });

  it('asks for the reduced-motion query specifically', () => {
    const media = stubMatchMedia(false);

    renderHook(() => useReducedMotion());

    expect(media.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
  });

  it('follows a preference change during the session', () => {
    const media = stubMatchMedia(false);
    const view = renderHook(() => useReducedMotion());
    expect(view.result.current).toBe(false);

    act(() => {
      media.change(true);
    });

    expect(view.result.current).toBe(true);
  });

  it('unsubscribes on unmount', () => {
    const media = stubMatchMedia(false);
    const view = renderHook(() => useReducedMotion());

    view.unmount();

    expect(media.removeEventListener).toHaveBeenCalled();
  });

  it('reports no preference when matchMedia is unavailable', () => {
    vi.stubGlobal('matchMedia', undefined);

    expect(renderHook(() => useReducedMotion()).result.current).toBe(false);
  });
});

describe('useReducedMotion on native', () => {
  it('starts unresolved while the platform query is pending', () => {
    platform.OS = 'ios';
    isReduceMotionEnabled.mockReturnValue(new Promise(() => undefined));
    addEventListener.mockReturnValue({ remove: vi.fn() });

    expect(renderHook(() => useReducedMotionPreference()).result.current).toEqual({
      reducedMotion: false,
      resolved: false,
    });
  });

  it.each(['ios', 'android'])('resolves and observes the platform setting on %s', async (os) => {
    platform.OS = os;
    isReduceMotionEnabled.mockResolvedValue(true);
    let listener: ((enabled: boolean) => void) | undefined;
    const remove = vi.fn();
    addEventListener.mockImplementation((_event, next) => {
      listener = next;
      return { remove };
    });

    const view = renderHook(() => useReducedMotionPreference());

    await vi.waitFor(() => {
      expect(view.result.current).toEqual({ reducedMotion: true, resolved: true });
    });
    expect(addEventListener).toHaveBeenCalledWith('reduceMotionChanged', expect.any(Function));

    act(() => {
      listener?.(false);
    });
    expect(view.result.current).toEqual({ reducedMotion: false, resolved: true });

    view.unmount();
    expect(remove).toHaveBeenCalled();
  });

  it('resolves after a rejected platform query', async () => {
    platform.OS = 'ios';
    isReduceMotionEnabled.mockRejectedValue(new Error('query unavailable'));
    addEventListener.mockReturnValue({ remove: vi.fn() });

    const view = renderHook(() => useReducedMotionPreference());

    await vi.waitFor(() => {
      expect(view.result.current).toEqual({ reducedMotion: false, resolved: true });
    });
  });

  it('does not update after unmount when a slow read resolves', async () => {
    platform.OS = 'ios';
    let resolveQuery: ((enabled: boolean) => void) | undefined;
    isReduceMotionEnabled.mockReturnValue(
      new Promise((resolve) => {
        resolveQuery = resolve;
      }),
    );
    const remove = vi.fn();
    addEventListener.mockReturnValue({ remove });
    let renders = 0;

    const view = renderHook(() => {
      renders += 1;
      return useReducedMotionPreference();
    });
    view.unmount();
    await act(async () => {
      resolveQuery?.(true);
      await Promise.resolve();
    });

    expect(renders).toBe(1);
    expect(remove).toHaveBeenCalledOnce();
  });
});
