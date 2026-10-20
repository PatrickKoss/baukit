// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import type { RefObject } from 'react';
import type { View } from 'react-native';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

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

function ref(current: unknown): RefObject<View | null> {
  return { current } as RefObject<View | null>;
}

function flushInteractions(): void {
  const tasks = pendingInteractions.splice(0, pendingInteractions.length);
  for (const task of tasks) task();
}

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.clearAllTimers();
  vi.useRealTimers();
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

  it('resolves the fallback when the trigger unmounts while open', () => {
    const fallback = { focus: vi.fn() };
    findNodeHandle.mockImplementation((target) =>
      target === TRIGGER ? 3 : target === fallback ? 9 : null,
    );
    const triggerRef = ref(TRIGGER);
    const view = renderHook(() =>
      useOverlayA11y({
        active: true,
        containerRef: ref(CONTAINER),
        triggerRef,
        returnFocus: () => fallback,
      }),
    );

    triggerRef.current = null;
    view.unmount();

    expect(setAccessibilityFocus).toHaveBeenCalledExactlyOnceWith(9);
  });

  it.each([false, true])('honors declined restoration with fallback = %s', (withFallback) => {
    const fallback = { focus: vi.fn() };
    findNodeHandle.mockImplementation((target) =>
      target === TRIGGER ? 3 : target === fallback ? 9 : null,
    );
    const view = renderHook(() =>
      useOverlayA11y({
        active: true,
        containerRef: ref(CONTAINER),
        triggerRef: ref(TRIGGER),
        restoreFocus: () => false,
        returnFocus: withFallback ? () => fallback : undefined,
      }),
    );

    view.unmount();

    if (withFallback) expect(setAccessibilityFocus).toHaveBeenCalledExactlyOnceWith(9);
    else expect(setAccessibilityFocus).not.toHaveBeenCalled();
  });

  it('reads the latest restoration callbacks without repeating native entry focus', () => {
    const fallback = { focus: vi.fn() };
    findNodeHandle.mockImplementation((target) =>
      target === CONTAINER ? 7 : target === TRIGGER ? 3 : target === fallback ? 9 : null,
    );
    const containerRef = ref(CONTAINER);
    const triggerRef = ref(TRIGGER);
    const view = renderHook(
      ({ restoreFocus }) =>
        useOverlayA11y({
          active: true,
          containerRef,
          triggerRef,
          restoreFocus,
          returnFocus: () => fallback,
        }),
      { initialProps: { restoreFocus: () => true } },
    );
    act(flushInteractions);
    setAccessibilityFocus.mockReset();

    view.rerender({ restoreFocus: () => false });
    expect(pendingInteractions).toHaveLength(0);
    expect(setAccessibilityFocus).not.toHaveBeenCalled();
    view.unmount();

    expect(setAccessibilityFocus).toHaveBeenCalledExactlyOnceWith(9);
  });

  it('honors an opt-out supplied in the render that closes the native overlay', () => {
    findNodeHandle.mockReturnValue(3);
    const containerRef = ref(CONTAINER);
    const view = renderHook(
      ({ active }) =>
        useOverlayA11y({
          active,
          containerRef,
          triggerHandle: 3,
          restoreFocus: () => active,
        }),
      { initialProps: { active: true } },
    );

    view.rerender({ active: false });

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
  function mountOverlay() {
    platform.OS = 'web';
    document.body.innerHTML = `
      <button id="opener">open</button>
      <div id="overlay"><button id="inside">inside</button></div>
      <div id="outside"><h1 id="destination" tabindex="-1">destination</h1></div>
    `;
    const opener = document.querySelector<HTMLButtonElement>('#opener');
    const overlay = document.querySelector<HTMLElement>('#overlay');
    const outside = document.querySelector<HTMLElement>('#outside');
    const destination = document.querySelector<HTMLElement>('#destination');
    if (!opener || !overlay || !outside || !destination) throw new Error('Missing overlay fixture');
    opener.focus();
    return { opener, overlay, outside, destination };
  }

  it('keeps outside focus moved after useInert releases the background', () => {
    const { overlay, destination } = mountOverlay();
    const view = renderHook(() => useOverlayA11y({ active: true, containerRef: ref(overlay) }));

    view.unmount();
    destination.focus();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(document.activeElement).toBe(destination);
  });

  it.each(['body', 'closing container'])(
    'restores focus from %s after releasing the inert opener',
    (location) => {
      const { opener, overlay } = mountOverlay();
      const view = renderHook(() => useOverlayA11y({ active: true, containerRef: ref(overlay) }));
      expect(opener.hasAttribute('inert')).toBe(true);

      view.unmount();
      if (location === 'body') overlay.remove();
      expect(opener.hasAttribute('inert')).toBe(false);
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it.each(['inert', 'hidden', 'aria-hidden'])(
    'restores when the outside destination becomes %s',
    (attribute) => {
      const { opener, overlay, outside, destination } = mountOverlay();
      const view = renderHook(() => useOverlayA11y({ active: true, containerRef: ref(overlay) }));

      view.unmount();
      destination.focus();
      outside.setAttribute(attribute, 'true');
      expect(document.activeElement).toBe(destination);
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it('uses the fallback when the opener disconnects', () => {
    const { opener, overlay, destination } = mountOverlay();
    const view = renderHook(() =>
      useOverlayA11y({
        active: true,
        containerRef: ref(overlay),
        returnFocus: () => destination,
      }),
    );

    view.unmount();
    opener.remove();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(document.activeElement).toBe(destination);
  });

  it('does nothing when restoration is declined', () => {
    const { opener, overlay } = mountOverlay();
    const focus = vi.spyOn(opener, 'focus');
    const view = renderHook(() =>
      useOverlayA11y({ active: true, containerRef: ref(overlay), restoreFocus: () => false }),
    );

    view.unmount();
    overlay.remove();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(focus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(document.body);
  });

  it('cancels restoration when reopened before the next task', () => {
    const { opener, overlay } = mountOverlay();
    const containerRef = ref(overlay);
    const focus = vi.spyOn(opener, 'focus');
    const view = renderHook(({ active }) => useOverlayA11y({ active, containerRef }), {
      initialProps: { active: true },
    });

    view.rerender({ active: false });
    view.rerender({ active: true });
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(focus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(overlay.firstElementChild);
  });

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
