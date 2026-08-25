// @vitest-environment jsdom
import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { HostRef, QueryRoot, TreeElement } from './dom-boundary.js';
import {
  ARIA_HIDDEN_INERT_OPT_OUT,
  makeOutsideSiblingsInert,
  syncAriaHiddenInert,
  useAriaHiddenInert,
  useInert,
} from './use-inert.js';

const MANAGED = 'data-baukit-aria-hidden-inert';

function element(tag = 'div', id?: string): HTMLElement {
  const node = document.createElement(tag);
  if (id !== undefined) node.id = id;
  return node;
}

function asTree(node: HTMLElement): TreeElement {
  return node;
}

function treeById(id: string): TreeElement {
  const element = document.querySelector<HTMLElement>(`#${id}`);
  if (element === null) throw new Error(`no element with id ${id}`);
  return asTree(element);
}

function ref(node: HTMLElement | null): HostRef {
  return { current: node };
}

function bodyRoot(): QueryRoot {
  return document.body;
}

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

describe('syncAriaHiddenInert', () => {
  it('adds inert to an aria-hidden element and marks it as managed', () => {
    const hidden = element('div', 'scene');
    hidden.setAttribute('aria-hidden', 'true');
    document.body.appendChild(hidden);

    syncAriaHiddenInert(bodyRoot());

    expect(hidden.hasAttribute('inert')).toBe(true);
    expect(hidden.hasAttribute(MANAGED)).toBe(true);
  });

  it('marks the whole aria-hidden scene, not just its controls', () => {
    document.body.innerHTML = `
      <div id="scene" aria-hidden="true"><button id="behind">behind</button></div>
      <button id="front">front</button>
    `;

    syncAriaHiddenInert(bodyRoot());

    // jsdom 30 does not implement inert, so this asserts the attribute the
    // browser acts on. The real focus-blocking is a browser-level check.
    expect(document.querySelector('#scene')?.hasAttribute('inert')).toBe(true);
    expect(document.querySelector('#front')?.hasAttribute('inert')).toBe(false);
  });

  it('leaves an opted-out element focusable', () => {
    const hidden = element();
    hidden.setAttribute('aria-hidden', 'true');
    hidden.setAttribute(ARIA_HIDDEN_INERT_OPT_OUT, '');
    document.body.appendChild(hidden);

    syncAriaHiddenInert(bodyRoot());

    expect(hidden.hasAttribute('inert')).toBe(false);
  });

  it('releases inert once aria-hidden is gone', () => {
    const scene = element();
    scene.setAttribute('aria-hidden', 'true');
    document.body.appendChild(scene);
    syncAriaHiddenInert(bodyRoot());

    scene.removeAttribute('aria-hidden');
    syncAriaHiddenInert(bodyRoot());

    expect(scene.hasAttribute('inert')).toBe(false);
    expect(scene.hasAttribute(MANAGED)).toBe(false);
  });

  it('never removes inert that the product set itself', () => {
    const own = element();
    own.setAttribute('inert', '');
    document.body.appendChild(own);

    syncAriaHiddenInert(bodyRoot());

    expect(own.hasAttribute('inert')).toBe(true);
    expect(own.hasAttribute(MANAGED)).toBe(false);
  });
});

describe('makeOutsideSiblingsInert', () => {
  it('marks every ancestor sibling inert and restores them on release', () => {
    document.body.innerHTML = `
      <div id="middle"><div id="overlay"></div><div id="sibling"></div></div>
      <div id="uncle"></div>
    `;
    const overlay = document.querySelector<HTMLElement>('#overlay');
    const sibling = document.querySelector<HTMLElement>('#sibling');
    const uncle = document.querySelector<HTMLElement>('#uncle');
    const middle = document.querySelector<HTMLElement>('#middle');

    const release = makeOutsideSiblingsInert(treeById('overlay'));

    expect(sibling?.hasAttribute('inert')).toBe(true);
    expect(uncle?.hasAttribute('inert')).toBe(true);
    expect(overlay?.hasAttribute('inert')).toBe(false);
    expect(middle?.hasAttribute('inert')).toBe(false);

    release();

    expect(sibling?.hasAttribute('inert')).toBe(false);
    expect(uncle?.hasAttribute('inert')).toBe(false);
  });

  it('marks the background but never the overlay it protects', () => {
    document.body.innerHTML = `
      <div id="overlay"><button id="confirm">confirm</button></div>
      <button id="background">background</button>
    `;
    const overlay = document.querySelector<HTMLElement>('#overlay');

    makeOutsideSiblingsInert(treeById('overlay'));

    expect(document.querySelector('#background')?.hasAttribute('inert')).toBe(true);
    expect(overlay?.hasAttribute('inert')).toBe(false);
    expect(document.querySelector('#confirm')?.hasAttribute('inert')).toBe(false);

    // The overlay's own controls stay reachable.
    document.querySelector<HTMLElement>('#confirm')?.focus();
    expect(document.activeElement?.id).toBe('confirm');
  });

  it('keeps a sibling inert until the last overlay releases it', () => {
    document.body.innerHTML = `
      <div id="first"></div><div id="second"></div><div id="sibling"></div>
    `;
    const sibling = document.querySelector<HTMLElement>('#sibling');

    const releaseFirst = makeOutsideSiblingsInert(treeById('first'));
    const releaseSecond = makeOutsideSiblingsInert(treeById('second'));

    expect(sibling?.hasAttribute('inert')).toBe(true);
    releaseFirst();
    expect(sibling?.hasAttribute('inert')).toBe(true);
    releaseSecond();
    expect(sibling?.hasAttribute('inert')).toBe(false);
  });

  it('leaves a sibling that was already inert before the overlay opened', () => {
    document.body.innerHTML = '<div id="overlay"></div><div id="frozen" inert></div>';
    const frozen = document.querySelector<HTMLElement>('#frozen');

    makeOutsideSiblingsInert(treeById('overlay'))();

    expect(frozen?.hasAttribute('inert')).toBe(true);
  });

  it('stops climbing at the document body', () => {
    document.body.innerHTML = '<div id="overlay"></div>';
    const head = document.head;

    makeOutsideSiblingsInert(treeById('overlay'));

    expect(head.hasAttribute('inert')).toBe(false);
    expect(document.body.hasAttribute('inert')).toBe(false);
  });
});

describe('useInert', () => {
  it('makes the outside inert while active and releases it on unmount', () => {
    document.body.innerHTML = '<div id="overlay"></div><div id="sibling"></div>';
    const overlay = document.querySelector<HTMLElement>('#overlay');
    const sibling = document.querySelector<HTMLElement>('#sibling');

    const view = renderHook(() => {
      useInert(ref(overlay), true);
    });
    expect(sibling?.hasAttribute('inert')).toBe(true);

    view.unmount();
    expect(sibling?.hasAttribute('inert')).toBe(false);
  });

  it('does nothing while inactive', () => {
    document.body.innerHTML = '<div id="overlay"></div><div id="sibling"></div>';

    renderHook(() => {
      useInert(ref(document.querySelector('#overlay')), false);
    });

    expect(document.querySelector('#sibling')?.hasAttribute('inert')).toBe(false);
  });

  it('does nothing before the container mounts or when it has no parent', () => {
    expect(() => {
      renderHook(() => {
        useInert(ref(null), true);
      });
      renderHook(() => {
        useInert(ref(element()), true);
      });
    }).not.toThrow();
  });
});

describe('useAriaHiddenInert', () => {
  it('syncs on mount, follows mutations, and cleans up on unmount', async () => {
    const scene = element('div', 'scene');
    scene.setAttribute('aria-hidden', 'true');
    document.body.appendChild(scene);

    const view = renderHook(() => {
      useAriaHiddenInert();
    });
    expect(scene.hasAttribute('inert')).toBe(true);

    scene.removeAttribute('aria-hidden');
    await vi.waitFor(() => {
      expect(scene.hasAttribute('inert')).toBe(false);
    });

    scene.setAttribute('aria-hidden', 'true');
    await vi.waitFor(() => {
      expect(scene.hasAttribute('inert')).toBe(true);
    });

    view.unmount();
    expect(scene.hasAttribute('inert')).toBe(false);
    expect(scene.hasAttribute(MANAGED)).toBe(false);
  });

  it('picks up a scene added after mount', async () => {
    const view = renderHook(() => {
      useAriaHiddenInert();
    });

    const late = element('div', 'late');
    late.setAttribute('aria-hidden', 'true');
    document.body.appendChild(late);

    await vi.waitFor(() => {
      expect(late.hasAttribute('inert')).toBe(true);
    });
    view.unmount();
  });
});
