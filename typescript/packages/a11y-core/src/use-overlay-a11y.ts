import { useEffect, useRef, type RefObject } from 'react';
import {
  AccessibilityInfo,
  findNodeHandle,
  InteractionManager,
  Platform,
  type View,
  type ViewProps,
} from 'react-native';

import { hostElement } from './dom-boundary.js';
import { useFocusTrap, type FocusTrapProps } from './use-focus-trap.js';
import { useInert } from './use-inert.js';

/**
 * Runs `task` once the overlay has laid out and returns a canceller. Products
 * that already have a presentation event should pass their own.
 */
export type DeferFocus = (task: () => void) => { cancel: () => void };

export interface OverlayA11yOptions {
  active: boolean;
  containerRef: RefObject<View | null>;
  /** Overrides how the native focus request waits for layout. */
  deferFocus?: DeferFocus | undefined;
  /** Container whose outside siblings go inert. Defaults to `containerRef`. */
  inertContainerRef?: RefObject<View | null> | undefined;
  initialFocusRef?: RefObject<View | null> | undefined;
  onEscape?: (() => void) | undefined;
  /** Native tag of the opener, when the caller already resolved it. */
  triggerHandle?: number | null | undefined;
  triggerRef?: RefObject<View | null> | undefined;
}

export type OverlayBackgroundProps = Pick<
  ViewProps,
  'accessibilityElementsHidden' | 'importantForAccessibility'
>;

export interface OverlayA11yResult {
  backgroundProps: OverlayBackgroundProps;
  containerProps: FocusTrapProps;
}

// InteractionManager is deprecated in React Native 0.86 with no replacement for
// "after the overlay presented". It stays the default so existing callers keep
// working; pass `deferFocus` to drive focus from a real layout event instead.
const deferUntilInteractionsDone: DeferFocus = (task) =>
  // eslint-disable-next-line @typescript-eslint/no-deprecated
  InteractionManager.runAfterInteractions(task);

function nodeHandle(ref: RefObject<View | null> | undefined): number | null {
  const host = hostElement(ref);
  if (host === null) return null;
  try {
    return findNodeHandle(host as Parameters<typeof findNodeHandle>[0]);
  } catch {
    return null;
  }
}

/**
 * One contract for overlay focus on both platforms. Web traps Tab, closes on
 * Escape, and makes the background inert. Native moves accessibility focus into
 * the overlay after layout and returns it to the trigger on close.
 */
export function useOverlayA11y({
  active,
  containerRef,
  deferFocus = deferUntilInteractionsDone,
  inertContainerRef,
  initialFocusRef,
  onEscape,
  triggerHandle,
  triggerRef,
}: OverlayA11yOptions): OverlayA11yResult {
  const triggerHandleRef = useRef<number | null>(null);
  const containerProps = useFocusTrap({ active, containerRef, initialFocusRef, onEscape });
  useInert(inertContainerRef ?? containerRef, active);

  useEffect(() => {
    if (Platform.OS === 'web' || !active) return;

    triggerHandleRef.current = triggerHandle ?? nodeHandle(triggerRef);
    const task = deferFocus(() => {
      const containerHandle = nodeHandle(containerRef);
      if (containerHandle !== null) AccessibilityInfo.setAccessibilityFocus(containerHandle);
    });

    return () => {
      task.cancel();
      const openerHandle = triggerHandleRef.current;
      triggerHandleRef.current = null;
      if (openerHandle !== null) AccessibilityInfo.setAccessibilityFocus(openerHandle);
    };
  }, [active, containerRef, deferFocus, triggerHandle, triggerRef]);

  return {
    backgroundProps:
      Platform.OS !== 'web' && active
        ? { accessibilityElementsHidden: true, importantForAccessibility: 'no-hide-descendants' }
        : {},
    containerProps,
  };
}
