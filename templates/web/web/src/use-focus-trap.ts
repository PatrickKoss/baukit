import { useEffect, useRef, type RefObject } from 'react';

const FOCUSABLE_SELECTOR = [
  'a[href]',
  'button:not([disabled])',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

function isFocusable(element: HTMLElement): boolean {
  return (
    element.getAttribute('aria-disabled') !== 'true' &&
    element.getAttribute('aria-hidden') !== 'true' &&
    !element.closest('[inert]')
  );
}

export function focusableElements(container: HTMLElement): HTMLElement[] {
  return Array.from(container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter(
    isFocusable,
  );
}

export function wrapFocusAtBoundary(
  focusable: readonly HTMLElement[],
  activeElement: Element | null,
  movingBackward: boolean,
): boolean {
  const currentIndex = focusable.findIndex((element) => element === activeElement);
  const shouldWrap = movingBackward
    ? currentIndex <= 0
    : currentIndex === -1 || currentIndex === focusable.length - 1;
  if (!shouldWrap || focusable.length === 0) {
    return false;
  }

  focusable[movingBackward ? focusable.length - 1 : 0]?.focus({
    preventScroll: true,
  });
  return true;
}

interface FocusTrapOptions {
  readonly active: boolean;
  readonly containerRef: RefObject<HTMLElement | null>;
  readonly initialFocusRef?: RefObject<HTMLElement | null> | undefined;
  readonly onEscape: () => void;
}

export function useFocusTrap({
  active,
  containerRef,
  initialFocusRef,
  onEscape,
}: FocusTrapOptions): void {
  const onEscapeRef = useRef(onEscape);
  useEffect(() => {
    onEscapeRef.current = onEscape;
  }, [onEscape]);

  useEffect(() => {
    if (!active) {
      return;
    }

    const container = containerRef.current;
    if (container === null) {
      return;
    }
    const activeContainer: HTMLElement = container;

    const previouslyFocused =
      document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusInitial = () => {
      const initial =
        initialFocusRef?.current ?? focusableElements(activeContainer)[0] ?? activeContainer;
      initial.focus({ preventScroll: true });
    };

    focusInitial();

    function handleKeyDown(event: KeyboardEvent): void {
      if (event.key === 'Escape') {
        event.preventDefault();
        onEscapeRef.current();
        return;
      }
      if (event.key !== 'Tab') {
        return;
      }

      const focusable = focusableElements(activeContainer);
      if (focusable.length === 0) {
        event.preventDefault();
        activeContainer.focus({ preventScroll: true });
        return;
      }
      if (wrapFocusAtBoundary(focusable, document.activeElement, event.shiftKey)) {
        event.preventDefault();
      }
    }

    function containFocus(event: FocusEvent): void {
      if (!(event.target instanceof Node) || !activeContainer.contains(event.target)) {
        focusInitial();
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    document.addEventListener('focusin', containFocus);
    return () => {
      document.removeEventListener('keydown', handleKeyDown);
      document.removeEventListener('focusin', containFocus);
      previouslyFocused?.focus({ preventScroll: true });
    };
  }, [active, containerRef, initialFocusRef]);
}
