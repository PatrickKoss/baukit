// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-native', () => {
  throw new Error('the web reduced-motion hooks must not import react-native');
});

import { useReducedMotion, useReducedMotionPreference } from './use-reduced-motion-web.js';

type MediaListener = () => void;

/** jsdom has no matchMedia. Each call returns a fresh list that reads the current setting. */
function stubMatchMedia(initial: boolean) {
  let matches = initial;
  const listeners = new Set<MediaListener>();
  const matchMedia = vi.fn((query: string) => ({
    get matches() {
      return matches;
    },
    media: query,
    addEventListener: (_event: string, listener: MediaListener) => {
      listeners.add(listener);
    },
    removeEventListener: (_event: string, listener: MediaListener) => {
      listeners.delete(listener);
    },
  }));
  vi.stubGlobal('matchMedia', matchMedia);
  return {
    matchMedia,
    listenerCount: () => listeners.size,
    change(next: boolean) {
      matches = next;
      for (const listener of listeners) listener();
    },
  };
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('useReducedMotion on the web entry', () => {
  it('reads the media query on the first render', () => {
    const media = stubMatchMedia(true);

    const { result } = renderHook(() => useReducedMotion());

    expect(result.current).toBe(true);
    expect(media.matchMedia).toHaveBeenCalledWith('(prefers-reduced-motion: reduce)');
  });

  it('follows changes and unsubscribes on unmount', () => {
    const media = stubMatchMedia(false);
    const { result, unmount } = renderHook(() => useReducedMotion());

    act(() => {
      media.change(true);
    });
    expect(result.current).toBe(true);

    unmount();
    expect(media.listenerCount()).toBe(0);
  });

  it('reports no preference when matchMedia is missing', () => {
    const { result } = renderHook(() => useReducedMotionPreference());

    expect(result.current).toEqual({ reducedMotion: false, resolved: true });
  });
});

describe('useReducedMotionPreference on the web entry', () => {
  it('resolves on the first client render and keeps a stable object', () => {
    stubMatchMedia(true);
    const { result, rerender } = renderHook(() => useReducedMotionPreference());
    const first = result.current;

    rerender();

    expect(first).toEqual({ reducedMotion: true, resolved: true });
    expect(result.current).toBe(first);
  });

  it('hydrates from the server value, then resolves', () => {
    stubMatchMedia(true);
    const seen: unknown[] = [];

    renderHook(
      () => {
        const preference = useReducedMotionPreference();
        seen.push(preference);
        return preference;
      },
      { hydrate: true },
    );

    expect(seen[0]).toEqual({ reducedMotion: false, resolved: false });
    expect(seen.at(-1)).toEqual({ reducedMotion: true, resolved: true });
  });
});
