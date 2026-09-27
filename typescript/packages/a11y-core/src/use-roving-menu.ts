import { useCallback, useEffect, useRef, useState, type RefObject } from 'react';

import { activeFocusTarget, asFocusTarget } from './dom-boundary.js';
import { hasDocument } from './platform.js';

export interface RovingMenuOption {
  readonly disabled?: boolean | undefined;
  readonly selected?: boolean | undefined;
}

export interface RovingMenuOptions {
  readonly active: boolean;
  readonly options: readonly RovingMenuOption[];
}

export interface RovingMenuKeyEvent {
  nativeEvent: { key: string; target?: unknown };
  preventDefault: () => void;
}

export interface RovingMenuItemProps {
  readonly onKeyDown: (event: RovingMenuKeyEvent) => void;
  readonly ref: (node: object | null) => void;
  readonly tabIndex: 0 | -1;
}

export interface RovingMenuResult {
  /** The item holding the tab stop, or null when no item is enabled. */
  readonly activeIndex: number | null;
  /** A stable ref to the active item's host; its `current` is null when no item is enabled. */
  readonly initialFocusRef: RefObject<object | null>;
  readonly itemProps: (index: number) => RovingMenuItemProps;
}

type ItemRef = (node: object | null) => void;

function isEnabled(option: RovingMenuOption | undefined): boolean {
  return option !== undefined && option.disabled !== true;
}

function enabledIndexes(options: readonly RovingMenuOption[]): number[] {
  return options.flatMap((option, index) => (isEnabled(option) ? [index] : []));
}

function nextAfter(enabled: readonly number[], currentIndex: number): number | undefined {
  return enabled.find((index) => index > currentIndex) ?? enabled[0];
}

function previousBefore(enabled: readonly number[], currentIndex: number): number | undefined {
  const before = enabled.filter((index) => index < currentIndex);
  return before[before.length - 1] ?? enabled[enabled.length - 1];
}

/**
 * Maps an arrow, Home, or End key to the enabled item it moves to, or null to
 * ignore the key. Movement is relative to `currentIndex` even when that item is
 * disabled or gone, and wraps at both ends.
 */
export function nextEnabledMenuIndex(
  key: string,
  currentIndex: number,
  options: readonly RovingMenuOption[],
): number | null {
  const enabled = enabledIndexes(options);
  if (enabled.length === 0) return null;

  switch (key) {
    case 'ArrowDown':
    case 'ArrowRight':
      return nextAfter(enabled, currentIndex) ?? null;
    case 'ArrowLeft':
    case 'ArrowUp':
      return previousBefore(enabled, currentIndex) ?? null;
    case 'Home':
      return enabled[0] ?? null;
    case 'End':
      return enabled[enabled.length - 1] ?? null;
    default:
      return null;
  }
}

function initialMenuIndex(options: readonly RovingMenuOption[]): number | null {
  const selected = options.findIndex((option) => option.selected === true && isEnabled(option));
  if (selected >= 0) return selected;
  const firstEnabled = options.findIndex(isEnabled);
  return firstEnabled >= 0 ? firstEnabled : null;
}

function resolveMenuIndex(
  movedIndex: number | null,
  options: readonly RovingMenuOption[],
): number | null {
  if (movedIndex !== null && isEnabled(options[movedIndex])) return movedIndex;
  return initialMenuIndex(options);
}

/** True when the key came from a host nested inside the item, such as a field. */
function isNestedTarget(target: unknown, item: object | null): boolean {
  return typeof target === 'object' && target !== null && item !== null && target !== item;
}

function focusFellToDocument(): boolean {
  if (!hasDocument()) return false;
  return document.activeElement === null || document.activeElement === document.body;
}

/**
 * Keyboard and focus state for an action menu: one tab stop on an enabled item,
 * arrow keys that skip disabled items, and Home and End. It renders nothing,
 * invokes no action, and leaves Escape, focus containment, and restoration to
 * `useOverlayA11y`. Call it before `useOverlayA11y` and pass `initialFocusRef` on.
 */
export function useRovingMenu({ active, options }: RovingMenuOptions): RovingMenuResult {
  const itemNodes = useRef<(object | null)[]>([]);
  const itemRefs = useRef(new Map<number, ItemRef>());
  const initialFocusRef = useRef<object | null>(null);
  const focusedItemRemoved = useRef(false);
  const [movedIndex, setMovedIndex] = useState<number | null>(null);
  const [wasActive, setWasActive] = useState(active);

  if (wasActive !== active) {
    setWasActive(active);
    setMovedIndex(null);
  }

  const activeIndex = resolveMenuIndex(movedIndex, options);

  useEffect(() => {
    initialFocusRef.current =
      activeIndex === null ? null : (itemNodes.current[activeIndex] ?? null);
    if (!focusedItemRemoved.current) return;
    focusedItemRemoved.current = false;
    if (active && focusFellToDocument()) asFocusTarget(initialFocusRef)?.focus();
  });

  const itemRef = useCallback((index: number): ItemRef => {
    const cached = itemRefs.current.get(index);
    if (cached !== undefined) return cached;

    const attach: ItemRef = (node) => {
      const previous = itemNodes.current[index] ?? null;
      if (node === null && previous !== null && previous === activeFocusTarget()) {
        focusedItemRemoved.current = true;
      }
      itemNodes.current[index] = node;
    };
    itemRefs.current.set(index, attach);
    return attach;
  }, []);

  const onItemKeyDown = (event: RovingMenuKeyEvent, index: number) => {
    if (isNestedTarget(event.nativeEvent.target, itemNodes.current[index] ?? null)) return;
    const nextIndex = nextEnabledMenuIndex(event.nativeEvent.key, index, options);
    if (nextIndex === null) return;

    event.preventDefault();
    setMovedIndex(nextIndex);
    asFocusTarget({ current: itemNodes.current[nextIndex] ?? null })?.focus();
  };

  return {
    activeIndex,
    initialFocusRef,
    itemProps: (index: number) => ({
      onKeyDown: (event: RovingMenuKeyEvent) => {
        onItemKeyDown(event, index);
      },
      ref: itemRef(index),
      tabIndex: index === activeIndex ? 0 : -1,
    }),
  };
}
