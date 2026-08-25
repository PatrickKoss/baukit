// @vitest-environment jsdom
import type { RefObject } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  activeFocusTarget,
  asFocusContainer,
  asFocusTarget,
  asTreeElement,
  hostElement,
} from './dom-boundary.js';

function ref(current: unknown): RefObject<never> {
  return { current } as unknown as RefObject<never>;
}

afterEach(() => {
  vi.unstubAllGlobals();
  document.body.innerHTML = '';
});

describe('hostElement', () => {
  it('returns null for an undefined ref, an empty ref, and a non-object host', () => {
    expect(hostElement(undefined)).toBeNull();
    expect(hostElement(ref(null))).toBeNull();
    expect(hostElement(ref(42))).toBeNull();
  });

  it('returns the host element when one is attached', () => {
    const host = document.createElement('div');
    expect(hostElement(ref(host))).toBe(host);
  });
});

describe('capability narrowing', () => {
  it('accepts a real DOM element for every capability', () => {
    const element = document.createElement('button');
    document.body.appendChild(element);

    expect(asFocusTarget(ref(element))).toBe(element);
    expect(asFocusContainer(ref(element))).toBe(element);
    expect(asTreeElement(ref(element))).toBe(element);
  });

  it('rejects a native host that has no DOM capabilities', () => {
    const nativeView = { measure: () => undefined };

    expect(asFocusTarget(ref(nativeView))).toBeNull();
    expect(asFocusContainer(ref(nativeView))).toBeNull();
    expect(asTreeElement(ref(nativeView))).toBeNull();
  });

  it('rejects a container that can query but cannot receive focus', () => {
    expect(asFocusContainer(ref({ querySelectorAll: () => [] }))).toBeNull();
  });

  it('narrows a text node to nothing, since it has no element methods', () => {
    expect(asFocusTarget(ref(document.createTextNode('text')))).toBeNull();
    expect(asTreeElement(ref(document.createTextNode('text')))).toBeNull();
  });
});

describe('activeFocusTarget', () => {
  it('returns null without a document', () => {
    vi.stubGlobal('document', undefined);
    expect(activeFocusTarget()).toBeNull();
  });

  it('returns the body when nothing has been focused', () => {
    expect(activeFocusTarget()).toBe(document.body);
  });

  it('returns the focused element', () => {
    const button = document.createElement('button');
    document.body.appendChild(button);
    button.focus();

    expect(activeFocusTarget()).toBe(button);
  });
});
