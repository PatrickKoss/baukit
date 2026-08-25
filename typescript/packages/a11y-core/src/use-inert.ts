import { useEffect } from 'react';

import {
  asTreeElement,
  type AttributeHost,
  type HostRef,
  type QueryRoot,
  type TreeElement,
} from './dom-boundary.js';
import { hasDocument } from './platform.js';

const MANAGED_ARIA_HIDDEN_INERT = 'data-baukit-aria-hidden-inert';

/** Put this attribute on an element that must stay focusable while aria-hidden. */
export const ARIA_HIDDEN_INERT_OPT_OUT = 'data-baukit-inert-opt-out';

/**
 * Routers mark inactive web scenes aria-hidden, but aria-hidden alone leaves
 * their descendants in the keyboard focus order. This keeps inert in lockstep
 * without relying on React Native Web to forward an unknown DOM prop.
 */
export function syncAriaHiddenInert(root: QueryRoot): void {
  for (const element of root.querySelectorAll(
    `[aria-hidden="true"], [${MANAGED_ARIA_HIDDEN_INERT}]`,
  )) {
    const shouldBeInert =
      element.getAttribute('aria-hidden') === 'true' &&
      !element.hasAttribute(ARIA_HIDDEN_INERT_OPT_OUT);

    if (shouldBeInert) {
      if (!element.hasAttribute('inert')) {
        element.setAttribute('inert', '');
        element.setAttribute(MANAGED_ARIA_HIDDEN_INERT, '');
      }
    } else if (element.hasAttribute(MANAGED_ARIA_HIDDEN_INERT)) {
      element.removeAttribute(MANAGED_ARIA_HIDDEN_INERT);
      element.removeAttribute('inert');
    }
  }
}

/** Mirrors aria-hidden onto inert for the whole document while mounted. */
export function useAriaHiddenInert(): void {
  useEffect(() => {
    if (!hasDocument()) return;

    const root = document.body as unknown as QueryRoot;
    syncAriaHiddenInert(root);
    const observer = new MutationObserver(() => {
      syncAriaHiddenInert(root);
    });
    observer.observe(document.body, {
      attributeFilter: ['aria-hidden'],
      attributes: true,
      childList: true,
      subtree: true,
    });

    return () => {
      observer.disconnect();
      const managed: Iterable<AttributeHost> = root.querySelectorAll(
        `[${MANAGED_ARIA_HIDDEN_INERT}]`,
      );
      for (const element of managed) {
        element.removeAttribute(MANAGED_ARIA_HIDDEN_INERT);
        element.removeAttribute('inert');
      }
    };
  }, []);
}

interface InertState {
  count: number;
  wasInert: boolean;
}

const overlayInertStates = new WeakMap<object, InertState>();

function acquireInert(element: TreeElement): () => void {
  const current = overlayInertStates.get(element);
  if (current) {
    current.count += 1;
  } else {
    overlayInertStates.set(element, { count: 1, wasInert: element.hasAttribute('inert') });
    element.setAttribute('inert', '');
  }

  return () => {
    const state = overlayInertStates.get(element);
    if (!state) return;
    state.count -= 1;
    if (state.count > 0) return;
    overlayInertStates.delete(element);
    if (!state.wasInert) element.removeAttribute('inert');
  };
}

/** Marks every ancestor sibling of the container inert, preserving its own subtree. */
export function makeOutsideSiblingsInert(container: TreeElement): () => void {
  const releases: (() => void)[] = [];
  const body: unknown = typeof document === 'undefined' ? undefined : document.body;
  let branch: TreeElement = container;

  while (branch.parentElement !== null) {
    const parent: TreeElement = branch.parentElement;
    for (const sibling of Array.from(parent.children)) {
      if (sibling !== branch) releases.push(acquireInert(sibling));
    }
    branch = parent;
    if (body !== undefined && (parent as unknown) === body) break;
  }

  return () => {
    for (const release of releases.reverse()) release();
  };
}

/** Makes content outside a web overlay unavailable while the overlay is active. */
export function useInert(containerRef: HostRef, active: boolean): void {
  useEffect(() => {
    if (!hasDocument() || !active) return;
    const container = asTreeElement(containerRef);
    if (!container?.parentElement) return;
    return makeOutsideSiblingsInert(container);
  }, [active, containerRef]);
}
