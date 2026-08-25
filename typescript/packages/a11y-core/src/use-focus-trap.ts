import { useCallback, useEffect, useRef } from 'react';

import {
  activeFocusTarget,
  asFocusContainer,
  asFocusTarget,
  type FocusContainer,
  type FocusableElement,
  type FocusTarget,
  type HostRef,
} from './dom-boundary.js';
import { hasDocument } from './platform.js';

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[role="button"]:not([aria-disabled="true"])',
  '[role="link"]',
  '[role="menuitem"]:not([aria-disabled="true"])',
  '[role="radio"]:not([aria-disabled="true"])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

export interface FocusTrapKeyEvent {
  nativeEvent: { key: string; shiftKey?: boolean; target?: unknown };
  preventDefault(): void;
  stopPropagation?(): void;
}

export interface FocusTrapOptions {
  active: boolean;
  containerRef: HostRef;
  initialFocusRef?: HostRef | undefined;
  onEscape?: (() => void) | undefined;
}

export interface FocusTrapProps {
  onKeyDown: (event: FocusTrapKeyEvent) => void;
}

function isAvailable(element: FocusableElement): boolean {
  return (
    element.disabled !== true &&
    element.getAttribute('tabindex') !== '-1' &&
    element.getAttribute('aria-disabled') !== 'true' &&
    element.getAttribute('aria-hidden') !== 'true' &&
    !element.hasAttribute('inert')
  );
}

export function focusableElements(container: FocusContainer): FocusableElement[] {
  return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)).filter(isAvailable);
}

/** Moves focus to the far end when Tab leaves the trap, reporting whether it wrapped. */
export function wrapFocusAtBoundary(
  focusable: readonly FocusTarget[],
  activeElement: unknown,
  movingBackward: boolean,
): boolean {
  if (focusable.length === 0) return false;
  const currentIndex = focusable.findIndex((element) => element === activeElement);
  const shouldWrap = movingBackward
    ? currentIndex <= 0
    : currentIndex === -1 || currentIndex === focusable.length - 1;
  if (!shouldWrap) return false;
  focusable[movingBackward ? focusable.length - 1 : 0]?.focus({ preventScroll: true });
  return true;
}

/** Contains web Tab order inside an overlay and restores focus to its opener. */
/** Focuses the requested target, else the first focusable child, else the container. */
export function focusOverlayEntry(
  container: FocusContainer | null,
  requestedInitial: FocusTarget | null,
): void {
  const firstFocusable = container ? focusableElements(container)[0] : undefined;
  (requestedInitial ?? firstFocusable ?? container)?.focus({ preventScroll: true });
}

/**
 * Applies one key press to a trapped container. Returns `escape` when the caller
 * should close, `wrapped` when focus moved, and `ignored` otherwise.
 */
export function applyFocusTrapKey(
  container: FocusContainer | null,
  event: FocusTrapKeyEvent,
): 'escape' | 'wrapped' | 'ignored' {
  const key = event.nativeEvent.key;
  if (key === 'Escape') {
    event.preventDefault();
    event.stopPropagation?.();
    return 'escape';
  }
  if (key !== 'Tab' || !container) return 'ignored';

  const focusable = focusableElements(container);
  if (focusable.length === 0) {
    event.preventDefault();
    container.focus({ preventScroll: true });
    return 'wrapped';
  }

  const activeElement = activeFocusTarget() ?? event.nativeEvent.target;
  if (!wrapFocusAtBoundary(focusable, activeElement, event.nativeEvent.shiftKey === true)) {
    return 'ignored';
  }
  event.preventDefault();
  return 'wrapped';
}

export function useFocusTrap({
  active,
  containerRef,
  initialFocusRef,
  onEscape,
}: FocusTrapOptions): FocusTrapProps {
  const onEscapeRef = useRef(onEscape);
  const restoreTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    onEscapeRef.current = onEscape;
  }, [onEscape]);

  useEffect(() => {
    if (!hasDocument() || !active) return;
    if (restoreTimerRef.current !== undefined) {
      clearTimeout(restoreTimerRef.current);
      restoreTimerRef.current = undefined;
    }

    const previouslyFocused = activeFocusTarget();
    focusOverlayEntry(asFocusContainer(containerRef), asFocusTarget(initialFocusRef));

    return () => {
      // useInert releases outside siblings in a sibling effect cleanup. Restore on
      // the next task so the browser does not reject focus on a still-inert trigger.
      restoreTimerRef.current = setTimeout(() => {
        restoreTimerRef.current = undefined;
        previouslyFocused?.focus({ preventScroll: true });
      }, 0);
    };
  }, [active, containerRef, initialFocusRef]);

  const onKeyDown = useCallback(
    (event: FocusTrapKeyEvent) => {
      if (!hasDocument() || !active) return;
      if (applyFocusTrapKey(asFocusContainer(containerRef), event) === 'escape') {
        onEscapeRef.current?.();
      }
    },
    [active, containerRef],
  );

  return { onKeyDown };
}
