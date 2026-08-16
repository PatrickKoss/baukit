import { useEffect, type RefObject } from "react";

const MANAGED_INERT = "data-baukit-managed-inert";

interface InertState {
  count: number;
  wasInert: boolean;
}

const inertStates = new WeakMap<HTMLElement, InertState>();
const ariaHiddenReleases = new WeakMap<HTMLElement, () => void>();

function acquireInert(element: HTMLElement): () => void {
  const current = inertStates.get(element);
  if (current === undefined) {
    inertStates.set(element, {
      count: 1,
      wasInert: element.hasAttribute("inert"),
    });
    element.setAttribute("inert", "");
  } else {
    current.count += 1;
  }

  return () => {
    const state = inertStates.get(element);
    if (state === undefined) {
      return;
    }
    state.count -= 1;
    if (state.count > 0) {
      return;
    }
    inertStates.delete(element);
    if (!state.wasInert) {
      element.removeAttribute("inert");
    }
  };
}

function makeOutsideSiblingsInert(container: HTMLElement): () => void {
  const release: (() => void)[] = [];
  let branch = container;

  while (branch.parentElement !== null) {
    const parent: HTMLElement = branch.parentElement;
    for (const sibling of Array.from(parent.children)) {
      if (sibling instanceof HTMLElement && sibling !== branch) {
        release.push(acquireInert(sibling));
      }
    }
    branch = parent;
    if (parent === document.body) {
      break;
    }
  }

  return () => {
    for (const restore of release.reverse()) {
      restore();
    }
  };
}

/** Makes everything outside an active overlay unavailable to pointer and keyboard input. */
export function useInert(
  containerRef: RefObject<HTMLElement | null>,
  active: boolean,
): void {
  useEffect(() => {
    const container = containerRef.current;
    if (!active || container === null) {
      return;
    }
    return makeOutsideSiblingsInert(container);
  }, [active, containerRef]);
}

export function syncAriaHiddenInert(root: ParentNode): void {
  for (const element of root.querySelectorAll<HTMLElement>(
    `[aria-hidden="true"], [${MANAGED_INERT}]`,
  )) {
    if (element.getAttribute("aria-hidden") === "true") {
      if (!ariaHiddenReleases.has(element)) {
        ariaHiddenReleases.set(element, acquireInert(element));
        element.setAttribute(MANAGED_INERT, "");
      }
    } else if (element.hasAttribute(MANAGED_INERT)) {
      ariaHiddenReleases.get(element)?.();
      ariaHiddenReleases.delete(element);
      element.removeAttribute(MANAGED_INERT);
    }
  }
}

/** Keeps router scenes that use aria-hidden out of the focus order as well. */
export function useAriaHiddenInert(): void {
  useEffect(() => {
    syncAriaHiddenInert(document.body);
    const observer = new MutationObserver(() => {
      syncAriaHiddenInert(document.body);
    });
    observer.observe(document.body, {
      attributeFilter: ["aria-hidden"],
      attributes: true,
      childList: true,
      subtree: true,
    });
    return () => {
      observer.disconnect();
      for (const element of document.querySelectorAll<HTMLElement>(
        `[${MANAGED_INERT}]`,
      )) {
        ariaHiddenReleases.get(element)?.();
        ariaHiddenReleases.delete(element);
        element.removeAttribute(MANAGED_INERT);
      }
    };
  }, []);
}
