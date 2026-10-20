import { useCallback, useEffect, useRef } from 'react';

import {
  activeFocusTarget,
  asFocusContainer,
  asFocusTarget,
  hostElement,
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
  /** Whether to restore the opener. Defaults to true; false still allows `returnFocus`. */
  restoreFocus?: (() => boolean) | undefined;
  /** Fallback when the opener is gone or `restoreFocus` returns false. */
  returnFocus?: (() => FocusTarget | null) | undefined;
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

function hasOutsideFocus(container: object | null): boolean {
  const active = document.activeElement;
  if (
    active === null ||
    active === document.body ||
    !active.isConnected ||
    active.closest('[inert], [hidden], [aria-hidden="true"]') !== null
  ) {
    return false;
  }
  for (let element: Element | null = active; element !== null; element = element.parentElement) {
    if (element === container) return false;
  }
  return true;
}

function isConnected(target: FocusTarget | null | undefined): target is FocusTarget {
  return target != null && 'isConnected' in target && target.isConnected === true;
}

/** Contains web Tab order inside an overlay and restores focus when needed on close. */
export function useFocusTrap({
  active,
  containerRef,
  initialFocusRef,
  onEscape,
  restoreFocus,
  returnFocus,
}: FocusTrapOptions): FocusTrapProps {
  const onEscapeRef = useRef(onEscape);
  const restoreFocusRef = useRef(restoreFocus);
  const returnFocusRef = useRef(returnFocus);
  const restoreTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  useEffect(() => {
    onEscapeRef.current = onEscape;
  }, [onEscape]);

  useEffect(() => {
    restoreFocusRef.current = restoreFocus;
    returnFocusRef.current = returnFocus;
  }, [restoreFocus, returnFocus]);

  useEffect(() => {
    if (!hasDocument() || !active) return;
    if (restoreTimerRef.current !== undefined) {
      clearTimeout(restoreTimerRef.current);
      restoreTimerRef.current = undefined;
    }

    const previouslyFocused = activeFocusTarget();
    const container = hostElement(containerRef);
    focusOverlayEntry(asFocusContainer(containerRef), asFocusTarget(initialFocusRef));

    return () => {
      // useInert releases outside siblings in a sibling effect cleanup. Restore on
      // the next task so the browser does not reject focus on a still-inert trigger.
      restoreTimerRef.current = setTimeout(() => {
        restoreTimerRef.current = undefined;
        if (hasOutsideFocus(container)) return;
        const restoreOpener = restoreFocusRef.current?.() ?? true;
        const target =
          restoreOpener && isConnected(previouslyFocused)
            ? previouslyFocused
            : returnFocusRef.current?.();
        if (isConnected(target)) target.focus({ preventScroll: true });
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
