// @vitest-environment jsdom
import { act, cleanup, render, renderHook } from '@testing-library/react';
import { createElement, useRef } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { FocusContainer, FocusTarget, HostRef } from './dom-boundary.js';
import {
  applyFocusTrapKey,
  focusableElements,
  focusOverlayEntry,
  useFocusTrap,
  wrapFocusAtBoundary,
  type FocusTrapKeyEvent,
} from './use-focus-trap.js';

function mount(html: string): HTMLElement {
  const host = document.createElement('div');
  host.innerHTML = html;
  document.body.appendChild(host);
  return host;
}

/** React Native Web renders a View as this element; the tests supply it directly. */
function asContainer(element: HTMLElement): FocusContainer {
  return element;
}

function containerRef(element: HTMLElement | null): HostRef {
  return { current: element };
}

function ids(elements: readonly FocusTarget[]): string[] {
  return (elements as unknown as HTMLElement[]).map((element) => element.id);
}

function keyEvent(key: string, extras: { shiftKey?: boolean; target?: unknown } = {}) {
  const calls = { preventDefault: 0, stopPropagation: 0 };
  const event: FocusTrapKeyEvent & { calls: typeof calls } = {
    calls,
    nativeEvent: { key, ...extras },
    preventDefault: () => {
      calls.preventDefault += 1;
    },
    stopPropagation: () => {
      calls.stopPropagation += 1;
    },
  };
  return event;
}

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

describe('focusableElements', () => {
  it('drops elements the browser would skip', () => {
    const host = mount(`
      <button id="keep">keep</button>
      <button disabled>disabled</button>
      <button tabindex="-1">skipped</button>
      <button aria-disabled="true">aria-disabled</button>
      <button aria-hidden="true">aria-hidden</button>
      <button inert>inert</button>
    `);

    expect(ids(focusableElements(asContainer(host)))).toEqual(['keep']);
  });

  it('finds links, inputs, textareas, selects, and widget roles', () => {
    const host = mount(`
      <a href="#one" id="link">link</a>
      <input id="input" />
      <textarea id="textarea"></textarea>
      <select id="select"></select>
      <div role="button" tabindex="0" id="rolebutton"></div>
      <div role="menuitem" tabindex="0" id="menuitem"></div>
      <div role="radio" tabindex="0" id="radio"></div>
    `);

    expect(ids(focusableElements(asContainer(host)))).toEqual([
      'link',
      'input',
      'textarea',
      'select',
      'rolebutton',
      'menuitem',
      'radio',
    ]);
  });
});

describe('wrapFocusAtBoundary', () => {
  it('does nothing for an empty container', () => {
    expect(wrapFocusAtBoundary([], null, false)).toBe(false);
  });

  it('wraps forward from the last element to the first', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');
    host.querySelector<HTMLElement>('#b')?.focus();

    expect(
      wrapFocusAtBoundary(focusableElements(asContainer(host)), document.activeElement, false),
    ).toBe(true);
    expect(document.activeElement?.id).toBe('a');
  });

  it('wraps backward from the first element to the last', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');
    host.querySelector<HTMLElement>('#a')?.focus();

    expect(
      wrapFocusAtBoundary(focusableElements(asContainer(host)), document.activeElement, true),
    ).toBe(true);
    expect(document.activeElement?.id).toBe('b');
  });

  it('pulls an active element from outside the trap back in', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');
    const outside = mount('<button id="outside">outside</button>');
    (outside.firstElementChild as HTMLElement).focus();

    expect(
      wrapFocusAtBoundary(focusableElements(asContainer(host)), document.activeElement, false),
    ).toBe(true);
    expect(document.activeElement?.id).toBe('a');
  });

  it('leaves interior movement to the browser', () => {
    const host = mount(
      '<button id="a">a</button><button id="b">b</button><button id="c">c</button>',
    );
    host.querySelector<HTMLElement>('#b')?.focus();
    const ring = focusableElements(asContainer(host));

    expect(wrapFocusAtBoundary(ring, document.activeElement, false)).toBe(false);
    expect(wrapFocusAtBoundary(ring, document.activeElement, true)).toBe(false);
    expect(document.activeElement?.id).toBe('b');
  });
});

describe('focusOverlayEntry', () => {
  it('prefers the requested initial target', () => {
    const host = mount(
      '<button id="child">child</button><button id="requested">requested</button>',
    );
    const requested = host.querySelector('#requested') as unknown as FocusTarget;

    focusOverlayEntry(asContainer(host), requested);

    expect(document.activeElement?.id).toBe('requested');
  });

  it('falls back to the first focusable child', () => {
    const host = mount('<button id="first">first</button><button id="second">second</button>');

    focusOverlayEntry(asContainer(host), null);

    expect(document.activeElement?.id).toBe('first');
  });

  it('falls back to the container when it holds nothing focusable', () => {
    const host = mount('<p>no controls here</p>');
    host.id = 'shell';
    host.tabIndex = -1;

    focusOverlayEntry(asContainer(host), null);

    expect(document.activeElement?.id).toBe('shell');
  });

  it('tolerates a container that has not mounted', () => {
    expect(() => {
      focusOverlayEntry(null, null);
    }).not.toThrow();
  });
});

describe('applyFocusTrapKey', () => {
  it('reports Escape and stops the event from travelling further', () => {
    const host = mount('<button>only</button>');
    const event = keyEvent('Escape');

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('escape');
    expect(event.calls.preventDefault).toBe(1);
    expect(event.calls.stopPropagation).toBe(1);
  });

  it('ignores keys other than Tab and Escape', () => {
    const host = mount('<button>only</button>');
    const event = keyEvent('ArrowDown');

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('ignored');
    expect(event.calls.preventDefault).toBe(0);
  });

  it('ignores Tab before the container mounts', () => {
    expect(applyFocusTrapKey(null, keyEvent('Tab'))).toBe('ignored');
  });

  it('holds focus on an empty container instead of letting Tab escape', () => {
    const host = mount('<p>nothing focusable</p>');
    host.id = 'shell';
    host.tabIndex = -1;
    const event = keyEvent('Tab');

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('wrapped');
    expect(event.calls.preventDefault).toBe(1);
    expect(document.activeElement?.id).toBe('shell');
  });

  it('wraps at the end of the ring using the real active element', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');
    host.querySelector<HTMLElement>('#b')?.focus();
    const event = keyEvent('Tab');

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('wrapped');
    expect(event.calls.preventDefault).toBe(1);
    expect(document.activeElement?.id).toBe('a');
  });

  it('wraps backward past the first element', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');
    host.querySelector<HTMLElement>('#a')?.focus();
    const event = keyEvent('Tab', { shiftKey: true });

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('wrapped');
    expect(document.activeElement?.id).toBe('b');
  });

  it('leaves interior Tab movement alone', () => {
    const host = mount(
      '<button id="a">a</button><button id="b">b</button><button id="c">c</button>',
    );
    host.querySelector<HTMLElement>('#b')?.focus();
    const event = keyEvent('Tab');

    expect(applyFocusTrapKey(asContainer(host), event)).toBe('ignored');
    expect(event.calls.preventDefault).toBe(0);
    expect(document.activeElement?.id).toBe('b');
  });
});

describe('useFocusTrap', () => {
  it('moves focus to the first control when the overlay opens', () => {
    const host = mount('<button id="first">first</button><button id="second">second</button>');

    renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));

    expect(document.activeElement?.id).toBe('first');
  });

  it('restores focus to the opener on the next task', async () => {
    const opener = mount('<button id="opener">open</button>')
      .firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button id="inside">inside</button>');

    const view = renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));
    expect(document.activeElement?.id).toBe('inside');

    view.unmount();
    // The restore is deliberately deferred so useInert can release the trigger.
    expect(document.activeElement?.id).toBe('inside');

    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 1));
    });
    expect(document.activeElement?.id).toBe('opener');
  });

  it('prefers the requested initial focus target', () => {
    const host = mount('<button id="first">first</button><button id="wanted">wanted</button>');

    renderHook(() =>
      useFocusTrap({
        active: true,
        containerRef: containerRef(host),
        initialFocusRef: containerRef(host.querySelector('#wanted')),
      }),
    );

    expect(document.activeElement?.id).toBe('wanted');
  });

  it('does nothing while inactive', () => {
    const host = mount('<button id="first">first</button>');
    const onEscape = vi.fn();

    const view = renderHook(() =>
      useFocusTrap({ active: false, containerRef: containerRef(host), onEscape }),
    );
    act(() => {
      view.result.current.onKeyDown(keyEvent('Escape'));
    });

    expect(document.activeElement).toBe(document.body);
    expect(onEscape).not.toHaveBeenCalled();
  });

  it('traps a real Tab press at the end of the ring', () => {
    const host = mount('<button id="a">a</button><button id="b">b</button>');

    const view = renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));
    host.querySelector<HTMLElement>('#b')?.focus();
    const event = keyEvent('Tab');
    act(() => {
      view.result.current.onKeyDown(event);
    });

    expect(event.calls.preventDefault).toBe(1);
    expect(document.activeElement?.id).toBe('a');
  });

  it('reports Escape through the latest callback', () => {
    const host = mount('<button id="a">a</button>');
    const first = vi.fn();
    const second = vi.fn();

    const view = renderHook(
      ({ onEscape }: { onEscape: () => void }) =>
        useFocusTrap({ active: true, containerRef: containerRef(host), onEscape }),
      { initialProps: { onEscape: first } },
    );
    view.rerender({ onEscape: second });
    act(() => {
      view.result.current.onKeyDown(keyEvent('Escape'));
    });

    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });

  it('focuses the rendered dialog when React owns the ref', () => {
    function Dialog() {
      const ref = useRef<HTMLDivElement>(null);
      const props = useFocusTrap({ active: true, containerRef: ref });
      return createElement(
        'div',
        { ref, onKeyDown: props.onKeyDown },
        createElement('button', { id: 'confirm' }, 'confirm'),
      );
    }

    render(createElement(Dialog));

    expect(document.activeElement?.id).toBe('confirm');
  });
});
