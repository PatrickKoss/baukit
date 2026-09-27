// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { createElement, Fragment, useRef, useState, type RefObject } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'web' as string };
const setAccessibilityFocus = vi.fn<(tag: number) => void>();
const findNodeHandle = vi.fn<(target: object) => number | null>();

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
      task();
      return { cancel: vi.fn() };
    },
  },
}));

import * as root from './index.js';
import { useOverlayA11y } from './use-overlay-a11y.js';
import {
  nextEnabledMenuIndex,
  useRovingMenu,
  type RovingMenuKeyEvent,
  type RovingMenuOption,
} from './use-roving-menu.js';

interface Item extends RovingMenuOption {
  readonly label: string;
}

const ENABLED = {};
const DISABLED = { disabled: true };
const MIXED: readonly RovingMenuOption[] = [ENABLED, DISABLED, ENABLED, ENABLED];
const DISABLED_THEN_SELECTED: readonly Item[] = [
  { disabled: true, label: 'Disabled' },
  { label: 'Selected', selected: true },
  { label: 'Last' },
];
const EDIT_UNAVAILABLE_DELETE: readonly Item[] = [
  { label: 'Edit' },
  { disabled: true, label: 'Unavailable' },
  { label: 'Delete' },
];

function keyEvent(key: string, target?: unknown) {
  const preventDefault = vi.fn<() => void>();
  const event: RovingMenuKeyEvent = { nativeEvent: { key, target }, preventDefault };
  return { ...event, preventDefault };
}

function byId(id: string): HTMLElement {
  const element = document.getElementById(id);
  if (element === null) throw new Error(`no element with id ${id}`);
  return element;
}

function focusedId(): string | undefined {
  return document.activeElement?.id;
}

function tabStops(): string[] {
  return Array.from(document.querySelectorAll('[role="menuitem"][tabindex="0"]')).map(
    (item) => item.id,
  );
}

async function flushRestore(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

interface MenuProps {
  readonly items: readonly Item[];
  readonly nestedField?: boolean;
  readonly onAction: (item: Item) => void;
  readonly onClose: () => void;
  readonly open: boolean;
  readonly triggerRef: RefObject<HTMLButtonElement | null>;
}

function Menu({ items, nestedField = false, onAction, onClose, open, triggerRef }: MenuProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const menu = useRovingMenu({ active: open, options: items });
  const overlay = useOverlayA11y({
    active: open,
    containerRef: containerRef as never,
    initialFocusRef: menu.initialFocusRef,
    onEscape: onClose,
    triggerRef: triggerRef as never,
  });
  if (!open) return null;

  return createElement(
    'div',
    {
      ref: containerRef,
      role: 'dialog',
      tabIndex: -1,
      onKeyDown: overlay.containerProps.onKeyDown as never,
    },
    createElement('button', { id: 'close', onClick: onClose, type: 'button' }, 'Close'),
    createElement(
      'div',
      { role: 'menu' },
      items.map((item, index) => {
        const props = menu.itemProps(index);
        return createElement(
          'div',
          {
            'aria-disabled': item.disabled === true ? 'true' : undefined,
            id: item.label,
            key: item.label,
            onClick: () => {
              if (item.disabled !== true) onAction(item);
            },
            onKeyDown: props.onKeyDown as never,
            ref: props.ref,
            role: 'menuitem',
            tabIndex: props.tabIndex,
          },
          item.label,
          nestedField && index === 0 ? createElement('input', { id: 'nested-field' }) : null,
        );
      }),
    ),
  );
}

interface AppProps {
  readonly initialOpen?: boolean;
  readonly items: readonly Item[];
  readonly nestedField?: boolean;
  readonly onAction?: (item: Item, close: () => void, reopen: () => void) => void;
}

function App({ initialOpen = true, items, nestedField = false, onAction }: AppProps) {
  const [open, setOpen] = useState(initialOpen);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const close = () => {
    setOpen(false);
  };
  const reopen = () => {
    setOpen(true);
  };

  return createElement(
    Fragment,
    null,
    createElement(
      'button',
      { id: 'trigger', onClick: reopen, ref: triggerRef, type: 'button' },
      'Open',
    ),
    createElement(Menu, {
      items,
      nestedField,
      onAction: (item: Item) => {
        onAction?.(item, close, reopen);
      },
      onClose: close,
      open,
      triggerRef,
    }),
  );
}

function openFromTrigger(props: AppProps) {
  const view = render(createElement(App, { ...props, initialOpen: false }));
  byId('trigger').focus();
  fireEvent.click(byId('trigger'));
  return view;
}

afterEach(() => {
  cleanup();
  setAccessibilityFocus.mockReset();
  findNodeHandle.mockReset();
  platform.OS = 'web';
  document.body.innerHTML = '';
});

describe('nextEnabledMenuIndex', () => {
  it.each([
    ['ArrowDown', 0, 2],
    ['ArrowRight', 0, 2],
    ['ArrowUp', 2, 0],
    ['ArrowLeft', 2, 0],
    ['ArrowDown', 2, 3],
  ])('maps %s from %i to %i and skips disabled items', (key, current, expected) => {
    expect(nextEnabledMenuIndex(key, current, MIXED)).toBe(expected);
  });

  it('wraps around both ends', () => {
    expect(nextEnabledMenuIndex('ArrowDown', 3, MIXED)).toBe(0);
    expect(nextEnabledMenuIndex('ArrowUp', 0, MIXED)).toBe(3);
  });

  it('sends Home and End to the first and last enabled items', () => {
    const edgesDisabled = [DISABLED, ENABLED, ENABLED, DISABLED];

    expect(nextEnabledMenuIndex('Home', 2, edgesDisabled)).toBe(1);
    expect(nextEnabledMenuIndex('End', 1, edgesDisabled)).toBe(2);
  });

  it('moves relative to a current item that is disabled', () => {
    expect(nextEnabledMenuIndex('ArrowDown', 1, MIXED)).toBe(2);
    expect(nextEnabledMenuIndex('ArrowUp', 1, MIXED)).toBe(0);
  });

  it('moves from a removed current item as if it sat past the end', () => {
    expect(nextEnabledMenuIndex('ArrowDown', 9, MIXED)).toBe(0);
    expect(nextEnabledMenuIndex('ArrowUp', 9, MIXED)).toBe(3);
  });

  it('enters from no current item at the matching end', () => {
    expect(nextEnabledMenuIndex('ArrowDown', -1, MIXED)).toBe(0);
    expect(nextEnabledMenuIndex('ArrowUp', -1, MIXED)).toBe(3);
  });

  it('stays on the only enabled item', () => {
    expect(nextEnabledMenuIndex('ArrowDown', 1, [DISABLED, ENABLED])).toBe(1);
    expect(nextEnabledMenuIndex('ArrowUp', 1, [DISABLED, ENABLED])).toBe(1);
  });

  it('ignores every key when no item is enabled', () => {
    for (const key of ['ArrowDown', 'ArrowUp', 'Home', 'End']) {
      expect(nextEnabledMenuIndex(key, 0, [DISABLED, DISABLED])).toBeNull();
      expect(nextEnabledMenuIndex(key, 0, [])).toBeNull();
    }
  });

  it('ignores keys that are not menu movement', () => {
    for (const key of ['Tab', 'Enter', ' ', 'Escape', 'a']) {
      expect(nextEnabledMenuIndex(key, 0, MIXED)).toBeNull();
    }
  });

  it('matches a DOM menu vector with a disabled first item and a selected item', () => {
    const vector = DISABLED_THEN_SELECTED;

    expect(nextEnabledMenuIndex('End', 1, vector)).toBe(2);
    expect(nextEnabledMenuIndex('ArrowUp', 2, vector)).toBe(1);
    expect(nextEnabledMenuIndex('ArrowRight', 1, vector)).toBe(2);
    expect(nextEnabledMenuIndex('Home', 2, vector)).toBe(1);
    expect(nextEnabledMenuIndex('ArrowDown', 2, vector)).toBe(1);
  });
});

describe('useRovingMenu state', () => {
  it('puts the tab stop on the selected enabled item', () => {
    const view = renderHook(() =>
      useRovingMenu({ active: true, options: [ENABLED, { selected: true }, ENABLED] }),
    );

    expect(view.result.current.activeIndex).toBe(1);
    expect([0, 1, 2].map((index) => view.result.current.itemProps(index).tabIndex)).toEqual([
      -1, 0, -1,
    ]);
  });

  it('skips a selected item that is disabled', () => {
    const view = renderHook(() =>
      useRovingMenu({
        active: true,
        options: [DISABLED, { disabled: true, selected: true }, ENABLED],
      }),
    );

    expect(view.result.current.activeIndex).toBe(2);
  });

  it('gives a disabled-only menu no tab stop and no focus target', () => {
    const view = renderHook(() => useRovingMenu({ active: true, options: [DISABLED, DISABLED] }));

    expect(view.result.current.activeIndex).toBeNull();
    expect(view.result.current.itemProps(0).tabIndex).toBe(-1);
    expect(view.result.current.itemProps(1).tabIndex).toBe(-1);
    expect(view.result.current.initialFocusRef.current).toBeNull();

    const event = keyEvent('ArrowDown');
    act(() => {
      view.result.current.itemProps(0).onKeyDown(event);
    });
    expect(event.preventDefault).not.toHaveBeenCalled();
  });

  it('leaves Enter, Space, and Escape to the product and the overlay', () => {
    const view = renderHook(() => useRovingMenu({ active: true, options: MIXED }));

    for (const key of ['Enter', ' ', 'Escape', 'Tab']) {
      const event = keyEvent(key);
      act(() => {
        view.result.current.itemProps(0).onKeyDown(event);
      });
      expect(event.preventDefault).not.toHaveBeenCalled();
    }
    expect(view.result.current.activeIndex).toBe(0);
  });

  it('moves the tab stop without a mounted item host', () => {
    const view = renderHook(() => useRovingMenu({ active: true, options: MIXED }));

    const event = keyEvent('End');
    act(() => {
      view.result.current.itemProps(0).onKeyDown(event);
    });

    expect(event.preventDefault).toHaveBeenCalled();
    expect(view.result.current.activeIndex).toBe(3);
  });

  it('starts from the initial item again after the menu closes', () => {
    const view = renderHook(({ active }) => useRovingMenu({ active, options: MIXED }), {
      initialProps: { active: true },
    });
    act(() => {
      view.result.current.itemProps(0).onKeyDown(keyEvent('End'));
    });
    expect(view.result.current.activeIndex).toBe(3);

    view.rerender({ active: false });
    view.rerender({ active: true });

    expect(view.result.current.activeIndex).toBe(0);
  });

  it('falls back to the initial item when the active item becomes disabled or disappears', () => {
    const view = renderHook(({ options }) => useRovingMenu({ active: true, options }), {
      initialProps: { options: MIXED },
    });
    act(() => {
      view.result.current.itemProps(0).onKeyDown(keyEvent('End'));
    });

    view.rerender({ options: [ENABLED, DISABLED, ENABLED, DISABLED] });
    expect(view.result.current.activeIndex).toBe(0);

    act(() => {
      view.result.current.itemProps(0).onKeyDown(keyEvent('End'));
    });
    expect(view.result.current.activeIndex).toBe(2);

    view.rerender({ options: [ENABLED, DISABLED] });
    expect(view.result.current.activeIndex).toBe(0);
  });

  it('keeps one initial focus ref across renders and option changes', () => {
    const view = renderHook(({ options }) => useRovingMenu({ active: true, options }), {
      initialProps: { options: MIXED },
    });
    const first = view.result.current.initialFocusRef;

    view.rerender({ options: [DISABLED] });
    view.rerender({ options: MIXED });

    expect(view.result.current.initialFocusRef).toBe(first);
  });

  it('is exported from the package root', () => {
    expect(root.useRovingMenu).toBe(useRovingMenu);
    expect(root.nextEnabledMenuIndex).toBe(nextEnabledMenuIndex);
  });
});

describe('useRovingMenu with useOverlayA11y on web', () => {
  it('enters on the selected item and roves among enabled items only', () => {
    openFromTrigger({ items: DISABLED_THEN_SELECTED });

    expect(focusedId()).toBe('Selected');
    expect(tabStops()).toEqual(['Selected']);

    fireEvent.keyDown(byId('Selected'), { key: 'End' });
    expect(focusedId()).toBe('Last');
    expect(tabStops()).toEqual(['Last']);

    fireEvent.keyDown(byId('Last'), { key: 'ArrowDown' });
    expect(focusedId()).toBe('Selected');

    fireEvent.keyDown(byId('Selected'), { key: 'ArrowUp' });
    expect(focusedId()).toBe('Last');

    fireEvent.keyDown(byId('Last'), { key: 'Home' });
    expect(focusedId()).toBe('Selected');

    fireEvent.keyDown(byId('Selected'), { key: ' ' });
    expect(focusedId()).toBe('Selected');
  });

  it('skips a disabled middle item in both directions', () => {
    openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE });

    expect(focusedId()).toBe('Edit');
    fireEvent.keyDown(byId('Edit'), { key: 'ArrowDown' });
    expect(focusedId()).toBe('Delete');
    fireEvent.keyDown(byId('Delete'), { key: 'ArrowUp' });
    expect(focusedId()).toBe('Edit');
  });

  it('moves focus from React Native style key events on host refs', () => {
    const view = renderHook(() =>
      useRovingMenu({ active: true, options: EDIT_UNAVAILABLE_DELETE }),
    );
    render(
      createElement(
        'div',
        null,
        EDIT_UNAVAILABLE_DELETE.map((item) =>
          createElement('div', { id: item.label, key: item.label, tabIndex: -1 }, item.label),
        ),
      ),
    );
    EDIT_UNAVAILABLE_DELETE.forEach((item, index) => {
      view.result.current.itemProps(index).ref(byId(item.label));
    });
    byId('Edit').focus();

    const event = keyEvent('ArrowDown');
    act(() => {
      view.result.current.itemProps(0).onKeyDown(event);
    });

    expect(event.preventDefault).toHaveBeenCalled();
    expect(focusedId()).toBe('Delete');
  });

  it('focuses the first focusable control when every item is disabled', () => {
    openFromTrigger({
      items: [
        { disabled: true, label: 'One' },
        { disabled: true, label: 'Two' },
      ],
    });

    expect(focusedId()).toBe('close');
    expect(tabStops()).toEqual([]);

    const arrow = fireEvent.keyDown(byId('One'), { key: 'ArrowDown' });
    expect(arrow).toBe(true);
    expect(focusedId()).toBe('close');
  });

  it('closes on Escape from an item without invoking it and restores the trigger', async () => {
    const onAction = vi.fn();
    openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE, onAction });
    fireEvent.keyDown(byId('Edit'), { key: 'ArrowDown' });

    fireEvent.keyDown(byId('Delete'), { key: 'Escape' });
    await flushRestore();

    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(onAction).not.toHaveBeenCalled();
    expect(focusedId()).toBe('trigger');
  });

  it('reopens on the initial item after an Escape dismissal', async () => {
    openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE });
    fireEvent.keyDown(byId('Edit'), { key: 'End' });
    fireEvent.keyDown(byId('Delete'), { key: 'Escape' });
    await flushRestore();

    fireEvent.click(byId('trigger'));

    expect(focusedId()).toBe('Edit');
    expect(tabStops()).toEqual(['Edit']);
  });

  it('moves focus to the next item when the focused item is removed', () => {
    const items: readonly Item[] = [{ label: 'A' }, { label: 'B' }, { label: 'C' }];
    const view = openFromTrigger({ items });
    fireEvent.keyDown(byId('A'), { key: 'ArrowDown' });
    expect(focusedId()).toBe('B');

    view.rerender(
      createElement(App, { initialOpen: false, items: [{ label: 'A' }, { label: 'C' }] }),
    );

    expect(focusedId()).toBe('C');
    expect(tabStops()).toEqual(['C']);
  });

  it('falls back to the initial item when the focused last item is removed', () => {
    const items: readonly Item[] = [{ label: 'A' }, { label: 'B' }, { label: 'C' }];
    const view = openFromTrigger({ items });
    fireEvent.keyDown(byId('A'), { key: 'End' });
    expect(focusedId()).toBe('C');

    view.rerender(
      createElement(App, { initialOpen: false, items: [{ label: 'A' }, { label: 'B' }] }),
    );

    expect(focusedId()).toBe('A');
    expect(tabStops()).toEqual(['A']);
  });

  it('leaves focus alone when an unfocused item is removed', () => {
    const items: readonly Item[] = [{ label: 'A' }, { label: 'B' }, { label: 'C' }];
    const view = openFromTrigger({ items });
    fireEvent.keyDown(byId('A'), { key: 'End' });

    view.rerender(
      createElement(App, { initialOpen: false, items: [{ label: 'B' }, { label: 'C' }] }),
    );

    expect(focusedId()).toBe('C');
    fireEvent.keyDown(byId('C'), { key: 'ArrowUp' });
    expect(focusedId()).toBe('B');
  });

  it('still restores the trigger after every item becomes disabled while open', async () => {
    const view = openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE });
    const allDisabled = EDIT_UNAVAILABLE_DELETE.map((item) => ({ ...item, disabled: true }));

    view.rerender(createElement(App, { initialOpen: false, items: allDisabled }));
    expect(tabStops()).toEqual([]);

    fireEvent.keyDown(byId('Edit'), { key: 'Escape' });
    await flushRestore();

    expect(focusedId()).toBe('trigger');
  });

  it('ignores movement keys from a field nested inside an item', () => {
    openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE, nestedField: true });
    byId('nested-field').focus();

    const notCancelled = fireEvent.keyDown(byId('nested-field'), { key: 'End' });

    expect(notCancelled).toBe(true);
    expect(focusedId()).toBe('nested-field');
    expect(tabStops()).toEqual(['Edit']);
  });

  it('keeps working after a product action fails and reopens the menu', async () => {
    const failures: unknown[] = [];
    openFromTrigger({
      items: EDIT_UNAVAILABLE_DELETE,
      onAction: (_item, close, reopen) => {
        close();
        void Promise.reject(new Error('delete failed')).catch((error: unknown) => {
          failures.push(error);
          reopen();
        });
      },
    });
    fireEvent.keyDown(byId('Edit'), { key: 'End' });

    fireEvent.click(byId('Delete'));
    await flushRestore();

    expect(failures).toHaveLength(1);
    expect(tabStops()).toEqual(['Edit']);
    fireEvent.keyDown(byId('Edit'), { key: 'ArrowDown' });
    expect(focusedId()).toBe('Delete');
  });

  it('survives the route unmounting the open menu', async () => {
    const view = openFromTrigger({ items: EDIT_UNAVAILABLE_DELETE });
    const detachedItem = byId('Edit');
    fireEvent.keyDown(detachedItem, { key: 'ArrowDown' });

    view.unmount();
    await flushRestore();

    expect(document.querySelector('[role="menu"]')).toBeNull();
    expect(document.querySelector('[inert]')).toBeNull();
    expect(() => fireEvent.keyDown(detachedItem, { key: 'ArrowDown' })).not.toThrow();
  });

  it('ignores a stale item handler that runs after unmount', () => {
    const view = renderHook(() => useRovingMenu({ active: true, options: MIXED }));
    const { onKeyDown } = view.result.current.itemProps(0);
    view.unmount();

    const event = keyEvent('ArrowDown');
    expect(() => {
      onKeyDown(event);
    }).not.toThrow();
  });
});

describe('useRovingMenu with useOverlayA11y on native', () => {
  const CONTAINER = { id: 'container' };
  const TRIGGER = { id: 'trigger' };

  function nativeRef(current: object | null): RefObject<never> {
    return { current } as unknown as RefObject<never>;
  }

  it('returns accessibility focus to the trigger when back closes the menu', () => {
    platform.OS = 'android';
    findNodeHandle.mockImplementation((target) =>
      target === CONTAINER ? 7 : target === TRIGGER ? 3 : null,
    );

    const view = renderHook(
      ({ active }) => {
        const menu = useRovingMenu({ active, options: MIXED });
        const overlay = useOverlayA11y({
          active,
          containerRef: nativeRef(CONTAINER),
          initialFocusRef: menu.initialFocusRef,
          triggerRef: nativeRef(TRIGGER),
        });
        return { menu, overlay };
      },
      { initialProps: { active: true } },
    );
    expect(setAccessibilityFocus).toHaveBeenLastCalledWith(7);
    expect(view.result.current.overlay.backgroundProps).toEqual({
      accessibilityElementsHidden: true,
      importantForAccessibility: 'no-hide-descendants',
    });

    act(() => {
      view.result.current.menu.itemProps(0).onKeyDown(keyEvent('End', 42));
    });
    expect(view.result.current.menu.activeIndex).toBe(3);

    view.rerender({ active: false });

    expect(setAccessibilityFocus).toHaveBeenLastCalledWith(3);
    expect(view.result.current.menu.activeIndex).toBe(0);
  });
});
