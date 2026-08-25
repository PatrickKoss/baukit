import type { RefObject } from 'react';

/**
 * React Native Web renders a `View` as a DOM element, but the `View` type never
 * says so. Every crossing from a ref to a DOM capability goes through this
 * module so the unchecked cast exists once instead of at each call site.
 */

/**
 * A ref to whatever the host rendered: a `View` instance under React Native, an
 * element under React DOM. Naming neither type keeps this module importable
 * from a product that has no React Native types installed.
 */
export type HostRef = RefObject<object | null>;

export interface FocusTarget {
  focus(options?: { preventScroll?: boolean }): void;
}

export interface AttributeHost {
  getAttribute(name: string): string | null;
  hasAttribute(name: string): boolean;
  removeAttribute(name: string): void;
  setAttribute(name: string, value: string): void;
}

export interface FocusableElement extends FocusTarget {
  disabled?: boolean;
  getAttribute(name: string): string | null;
  hasAttribute(name: string): boolean;
}

export interface QueryRoot {
  querySelectorAll(selector: string): Iterable<AttributeHost>;
}

export interface FocusContainer extends FocusableElement {
  querySelectorAll(selector: string): Iterable<FocusableElement>;
}

export interface TreeElement extends AttributeHost {
  children: ArrayLike<TreeElement>;
  parentElement: TreeElement | null;
}

function hasMethod<K extends string>(
  value: object,
  name: K,
): value is Record<K, (...args: never[]) => unknown> {
  return typeof (value as Record<string, unknown>)[name] === 'function';
}

/** Reads the host object behind a ref, or null when the ref is empty. */
export function hostElement(ref: HostRef | undefined): object | null {
  const current = ref?.current;
  return typeof current === 'object' && current !== null ? current : null;
}

export function asFocusTarget(ref: HostRef | undefined): FocusTarget | null {
  const host = hostElement(ref);
  return host !== null && hasMethod(host, 'focus') ? host : null;
}

export function asFocusContainer(ref: HostRef | undefined): FocusContainer | null {
  const host = hostElement(ref);
  if (host === null || !hasMethod(host, 'querySelectorAll') || !hasMethod(host, 'focus')) {
    return null;
  }
  return host as FocusContainer;
}

export function asTreeElement(ref: HostRef | undefined): TreeElement | null {
  const host = hostElement(ref);
  return host !== null && hasMethod(host, 'setAttribute') ? (host as TreeElement) : null;
}

/** The element that currently holds DOM focus, or null outside a browser document. */
export function activeFocusTarget(): FocusTarget | null {
  if (typeof document === 'undefined') return null;
  const active: unknown = document.activeElement;
  return typeof active === 'object' && active !== null && hasMethod(active, 'focus')
    ? active
    : null;
}
