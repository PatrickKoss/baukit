import { useEffect, useLayoutEffect, useRef, type RefObject } from 'react';
import {
  AccessibilityInfo,
  InteractionManager,
  Platform,
  type View,
  type ViewProps,
} from 'react-native';

import type { HostRef } from './dom-boundary.js';
import { nativeNodeHandle } from './native-focus.js';
import { useFocusTrap, type FocusTrapOptions, type FocusTrapProps } from './use-focus-trap.js';
import { useInert } from './use-inert.js';

/**
 * Runs `task` once the overlay has laid out and returns a canceller. Products
 * that already have a presentation event should pass their own.
 */
export type DeferFocus = (task: () => void) => { cancel: () => void };

export interface OverlayA11yOptions extends Pick<FocusTrapOptions, 'restoreFocus' | 'returnFocus'> {
  active: boolean;
  containerRef: RefObject<View | null>;
  /** Overrides how the native focus request waits for layout. */
  deferFocus?: DeferFocus | undefined;
  /** Container whose outside siblings go inert. Defaults to `containerRef`. */
  inertContainerRef?: RefObject<View | null> | undefined;
  initialFocusRef?: HostRef | undefined;
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
  restoreFocus,
  returnFocus,
  triggerHandle,
  triggerRef,
}: OverlayA11yOptions): OverlayA11yResult {
  const restoreFocusRef = useRef(restoreFocus);
  const returnFocusRef = useRef(returnFocus);
  useLayoutEffect(() => {
    restoreFocusRef.current = restoreFocus;
    returnFocusRef.current = returnFocus;
  }, [restoreFocus, returnFocus]);
  const containerProps = useFocusTrap({
    active,
    containerRef,
    initialFocusRef,
    onEscape,
    restoreFocus,
    returnFocus,
  });
  useInert(inertContainerRef ?? containerRef, active);

  useEffect(() => {
    if (Platform.OS === 'web' || !active) return;

    const task = deferFocus(() => {
      const containerHandle = nativeNodeHandle(containerRef);
      if (containerHandle !== null) AccessibilityInfo.setAccessibilityFocus(containerHandle);
    });

    return () => {
      task.cancel();
      const restoreOpener = restoreFocusRef.current?.() ?? true;
      const openerHandle = restoreOpener ? (triggerHandle ?? nativeNodeHandle(triggerRef)) : null;
      const targetHandle =
        openerHandle ?? nativeNodeHandle({ current: returnFocusRef.current?.() ?? null });
      if (targetHandle !== null) AccessibilityInfo.setAccessibilityFocus(targetHandle);
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
