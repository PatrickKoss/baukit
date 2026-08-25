// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import type { RefObject } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'ios' as string };
const setAccessibilityFocus = vi.fn<(tag: number) => void>();
const findNodeHandle = vi.fn<(target: object) => number | null>();
const pendingInteractions: (() => void)[] = [];
const cancel = vi.fn();

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
  AccessibilityInfo: {
    setAccessibilityFocus: (tag: number) => {
      setAccessibilityFocus(tag);
    },
  },
  findNodeHandle: (target: object) => findNodeHandle(target),
  InteractionManager: {
    runAfterInteractions: (task: () => void) => {
      pendingInteractions.push(task);
      return { cancel };
    },
  },
}));

import { useOverlayA11y } from './use-overlay-a11y.js';

const CONTAINER = { id: 'container' };
const TRIGGER = { id: 'trigger' };

function ref(current: unknown): RefObject<never> {
  return { current } as unknown as RefObject<never>;
}

function flushInteractions(): void {
  const tasks = pendingInteractions.splice(0, pendingInteractions.length);
  for (const task of tasks) task();
}

afterEach(() => {
  cleanup();
  setAccessibilityFocus.mockReset();
  findNodeHandle.mockReset();
  cancel.mockReset();
  pendingInteractions.length = 0;
  platform.OS = 'ios';
  document.body.innerHTML = '';
});

describe('useOverlayA11y on native', () => {
  it('waits for layout before moving accessibility focus into the overlay', () => {
    findNodeHandle.mockImplementation((target) => (target === CONTAINER ? 7 : null));

    renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(CONTAINER), triggerRef: ref(null) }),
    );

    expect(setAccessibilityFocus).not.toHaveBeenCalled();

    act(() => {
      flushInteractions();
    });

    expect(setAccessibilityFocus).toHaveBeenCalledWith(7);
  });

  it('returns focus to the trigger when the overlay closes', () => {
    findNodeHandle.mockImplementation((target) =>
      target === CONTAINER ? 7 : target === TRIGGER ? 3 : null,
    );

    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(CONTAINER), triggerRef: ref(TRIGGER) }),
    );
    act(() => {
      flushInteractions();
    });
    setAccessibilityFocus.mockReset();

    view.unmount();

    expect(cancel).toHaveBeenCalled();
    expect(setAccessibilityFocus).toHaveBeenCalledWith(3);
  });

  it('prefers a trigger handle the caller already resolved', () => {
    findNodeHandle.mockReturnValue(null);

    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(CONTAINER), triggerHandle: 42 }),
    );
    view.unmount();

    expect(setAccessibilityFocus).toHaveBeenCalledWith(42);
  });

  it('restores nothing when the trigger no longer exists', () => {
    findNodeHandle.mockReturnValue(null);

    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(CONTAINER), triggerRef: ref(null) }),
    );
    act(() => {
      flushInteractions();
    });
    view.unmount();

    expect(setAccessibilityFocus).not.toHaveBeenCalled();
  });

  it('survives a host node that disappears between layout and focus', () => {
    findNodeHandle.mockImplementation(() => {
      throw new Error('view unmounted');
    });

    expect(() => {
      renderHook(() =>
        useOverlayA11y({ active: true, containerRef: ref(CONTAINER), triggerRef: ref(TRIGGER) }),
      );
      act(() => {
        flushInteractions();
      });
    }).not.toThrow();
    expect(setAccessibilityFocus).not.toHaveBeenCalled();
  });

  it('hides the background from the accessibility tree only while active', () => {
    const open = renderHook(() => useOverlayA11y({ active: true, containerRef: ref(CONTAINER) }));
    const closed = renderHook(() =>
      useOverlayA11y({ active: false, containerRef: ref(CONTAINER) }),
    );

    expect(open.result.current.backgroundProps).toEqual({
      accessibilityElementsHidden: true,
      importantForAccessibility: 'no-hide-descendants',
    });
    expect(closed.result.current.backgroundProps).toEqual({});
  });

  it('schedules nothing while the overlay is closed', () => {
    renderHook(() => useOverlayA11y({ active: false, containerRef: ref(CONTAINER) }));

    expect(pendingInteractions).toHaveLength(0);
  });

  it('lets the caller drive focus from its own layout event', () => {
    findNodeHandle.mockReturnValue(9);
    let layout: (() => void) | undefined;
    const cancelLayout = vi.fn();
    const deferFocus = (task: () => void) => {
      layout = task;
      return { cancel: cancelLayout };
    };

    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(CONTAINER), deferFocus }),
    );

    expect(pendingInteractions).toHaveLength(0);
    expect(setAccessibilityFocus).not.toHaveBeenCalled();

    act(() => {
      layout?.();
    });
    expect(setAccessibilityFocus).toHaveBeenCalledWith(9);

    view.unmount();
    expect(cancelLayout).toHaveBeenCalled();
  });
});

describe('useOverlayA11y on web', () => {
  it('traps focus in the real overlay and leaves the background props empty', () => {
    platform.OS = 'web';
    document.body.innerHTML = `
      <div id="overlay"><button id="confirm">confirm</button></div>
      <button id="background">background</button>
    `;
    const overlay = document.querySelector<HTMLElement>('#overlay');
    const onEscape = vi.fn();

    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(overlay), onEscape }),
    );

    expect(view.result.current.backgroundProps).toEqual({});
    expect(pendingInteractions).toHaveLength(0);
    expect(document.activeElement?.id).toBe('confirm');
    expect(document.querySelector('#background')?.hasAttribute('inert')).toBe(true);

    act(() => {
      view.result.current.containerProps.onKeyDown({
        nativeEvent: { key: 'Escape' },
        preventDefault: vi.fn(),
      });
    });

    expect(onEscape).toHaveBeenCalledTimes(1);

    view.unmount();
    expect(document.querySelector('#background')?.hasAttribute('inert')).toBe(false);
  });
});
