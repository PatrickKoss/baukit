// @vitest-environment jsdom
import { act, cleanup, render, renderHook } from '@testing-library/react';
import { createElement, StrictMode, useRef, type RefObject } from 'react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { FocusContainer, FocusTarget, HostRef } from './dom-boundary.js';
import {
  applyFocusTrapKey,
  focusableElements,
  focusOverlayEntry,
  useFocusTrap,
  wrapFocusAtBoundary,
  type FocusTrapKeyEvent,
} from './web.js';

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

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.clearAllTimers();
  vi.useRealTimers();
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
  function Dialog({
    active = true,
    containerRef,
    initialFocusRef,
  }: {
    active?: boolean;
    containerRef?: RefObject<HTMLDivElement | null> | undefined;
    initialFocusRef?: RefObject<HTMLInputElement | null> | undefined;
  }) {
    const localRef = useRef<HTMLDivElement>(null);
    const ref = containerRef ?? localRef;
    useFocusTrap({ active, containerRef: ref, initialFocusRef });
    return active
      ? createElement(
          'div',
          { ref },
          createElement('button', { id: 'first' }, 'first'),
          createElement('input', { id: 'initial', ref: initialFocusRef }),
        )
      : null;
  }

  it.each([false, true])(
    'restores the opener after StrictMode effect replay with initialFocusRef = %s',
    (withInitialFocus) => {
      const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
      opener.focus();
      const initialFocusRef = withInitialFocus ? { current: null } : undefined;
      const view = render(
        createElement(StrictMode, null, createElement(Dialog, { initialFocusRef })),
      );
      expect(document.activeElement?.id).toBe(withInitialFocus ? 'initial' : 'first');

      view.unmount();
      expect(document.activeElement).toBe(document.body);
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it.each(['initialFocusRef', 'containerRef'] as const)(
    'keeps the opener when %s changes while open under StrictMode',
    (changedRef) => {
      const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
      opener.focus();
      const view = render(
        createElement(StrictMode, null, createElement(Dialog, { [changedRef]: { current: null } })),
      );
      const focus = vi.spyOn(opener, 'focus');

      view.rerender(
        createElement(StrictMode, null, createElement(Dialog, { [changedRef]: { current: null } })),
      );
      act(() => {
        vi.runOnlyPendingTimers();
      });
      expect(document.activeElement?.id).toBe(
        changedRef === 'initialFocusRef' ? 'initial' : 'first',
      );
      expect(focus).not.toHaveBeenCalled();

      view.unmount();
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it('captures a different opener for a separate opening under StrictMode', () => {
    const triggers = mount('<button>first opener</button><button>second opener</button>');
    const first = triggers.firstElementChild as HTMLButtonElement;
    const second = triggers.lastElementChild as HTMLButtonElement;
    first.focus();
    const view = render(createElement(StrictMode, null, createElement(Dialog)));

    view.rerender(createElement(StrictMode, null, createElement(Dialog, { active: false })));
    act(() => {
      vi.runOnlyPendingTimers();
    });
    expect(document.activeElement).toBe(first);

    second.focus();
    view.rerender(createElement(StrictMode, null, createElement(Dialog)));
    expect(document.activeElement?.id).toBe('first');
    view.rerender(createElement(StrictMode, null, createElement(Dialog, { active: false })));
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(document.activeElement).toBe(second);
  });

  it.each([false, true])(
    'does not reuse a previous opener after autoFocus with fallback = %s',
    (withFallback) => {
      const triggers = mount('<button>first opener</button><button>second opener</button>');
      const first = triggers.firstElementChild as HTMLButtonElement;
      const second = triggers.lastElementChild as HTMLButtonElement;
      const fallback = mount('<h1 tabindex="-1">fallback</h1>').firstElementChild as HTMLElement;

      function AutoFocusDialog({ active, autoFocus }: { active: boolean; autoFocus: boolean }) {
        const ref = useRef<HTMLDivElement>(null);
        useFocusTrap({
          active,
          containerRef: ref,
          returnFocus: withFallback ? () => fallback : undefined,
        });
        return active ? createElement('div', { ref }, createElement('input', { autoFocus })) : null;
      }

      first.focus();
      const view = render(createElement(AutoFocusDialog, { active: true, autoFocus: false }));
      view.rerender(createElement(AutoFocusDialog, { active: false, autoFocus: false }));
      act(() => {
        vi.runOnlyPendingTimers();
      });
      expect(document.activeElement).toBe(first);

      second.focus();
      view.rerender(createElement(AutoFocusDialog, { active: true, autoFocus: true }));
      expect(document.activeElement).toBe(view.container.querySelector('input'));
      view.unmount();
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(withFallback ? fallback : document.body);
    },
  );

  it('captures fresh focus after a declined restore under StrictMode', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button>inside</button>');
    const ref = containerRef(host);
    let restoreOpener = false;
    const view = renderHook(
      ({ active }) =>
        useFocusTrap({ active, containerRef: ref, restoreFocus: () => restoreOpener }),
      {
        initialProps: { active: true },
        wrapper: ({ children }) => createElement(StrictMode, null, children),
      },
    );

    view.rerender({ active: false });
    act(() => {
      vi.runOnlyPendingTimers();
    });
    expect(document.activeElement).toBe(host.firstElementChild);

    restoreOpener = true;
    view.rerender({ active: true });
    view.unmount();
    host.remove();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(document.activeElement).toBe(document.body);
  });

  it('moves focus to the first control when the overlay opens', () => {
    const host = mount('<button id="first">first</button><button id="second">second</button>');

    renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));

    expect(document.activeElement?.id).toBe('first');
  });

  it('restores focus to the opener on the next task', () => {
    const opener = mount('<button id="opener">open</button>')
      .firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button id="inside">inside</button>');

    const view = renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));
    expect(document.activeElement?.id).toBe('inside');

    view.unmount();
    // The restore is deliberately deferred so useInert can release the trigger.
    expect(document.activeElement?.id).toBe('inside');

    act(() => {
      vi.runOnlyPendingTimers();
    });
    expect(document.activeElement?.id).toBe('opener');
  });

  it('keeps focus moved outside after closing', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button>inside</button>');
    const destination = mount('<h1 tabindex="-1">new route</h1>').firstElementChild as HTMLElement;
    const restoreFocus = vi.fn(() => true);
    const returnFocus = vi.fn(() => opener);
    const view = renderHook(() =>
      useFocusTrap({ active: true, containerRef: containerRef(host), restoreFocus, returnFocus }),
    );

    view.unmount();
    destination.focus();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(document.activeElement).toBe(destination);
    expect(restoreFocus).not.toHaveBeenCalled();
    expect(returnFocus).not.toHaveBeenCalled();
  });

  it.each(['body', 'container', 'removed container'])(
    'restores when focus is on %s',
    (location) => {
      const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
      opener.focus();
      const host = mount('<button>inside</button>');
      host.tabIndex = -1;
      const ref = containerRef(host);
      const view = renderHook(() => useFocusTrap({ active: true, containerRef: ref }));

      view.unmount();
      ref.current = null;
      if (location === 'body') (document.activeElement as HTMLElement).blur();
      if (location === 'container') host.focus();
      if (location === 'removed container') host.remove();
      expect(document.activeElement).toBe(location === 'container' ? host : document.body);
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it.each(['inert', 'hidden', 'aria-hidden'])(
    'restores when outside focus is in a %s subtree',
    (attribute) => {
      const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
      opener.focus();
      const host = mount('<button>inside</button>');
      const outside = mount('<button>outside</button>');
      const destination = outside.firstElementChild as HTMLButtonElement;
      const view = renderHook(() =>
        useFocusTrap({ active: true, containerRef: containerRef(host) }),
      );

      view.unmount();
      destination.focus();
      outside.setAttribute(attribute, 'true');
      expect(document.activeElement).toBe(destination);
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(document.activeElement).toBe(opener);
    },
  );

  it('does not focus a disconnected opener', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const focus = vi.spyOn(opener, 'focus');
    const host = mount('<button>inside</button>');
    const view = renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));

    view.unmount();
    opener.remove();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(focus).not.toHaveBeenCalled();
  });

  it('resolves a fallback at restore time when the opener was disconnected', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button>inside</button>');
    let fallback: HTMLElement | null = null;
    const returnFocus = vi.fn(() => fallback);
    const view = renderHook(() =>
      useFocusTrap({ active: true, containerRef: containerRef(host), returnFocus }),
    );

    view.unmount();
    opener.remove();
    fallback = mount('<h1 tabindex="-1">fallback</h1>').firstElementChild as HTMLElement;
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(returnFocus).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(fallback);
  });

  it('does not focus a disconnected fallback', () => {
    const host = mount('<button>inside</button>');
    const fallback = document.createElement('button');
    const focus = vi.spyOn(fallback, 'focus');
    const view = renderHook(() =>
      useFocusTrap({
        active: true,
        containerRef: containerRef(host),
        restoreFocus: () => false,
        returnFocus: () => fallback,
      }),
    );

    view.unmount();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(focus).not.toHaveBeenCalled();
  });

  it('does nothing when restoration is declined without a fallback', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const focus = vi.spyOn(opener, 'focus');
    const host = mount('<button>inside</button>');
    const restoreFocus = vi.fn(() => false);
    const view = renderHook(() =>
      useFocusTrap({ active: true, containerRef: containerRef(host), restoreFocus }),
    );

    view.unmount();
    host.remove();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(restoreFocus).toHaveBeenCalledTimes(1);
    expect(focus).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(document.body);
  });

  it('uses the latest restoration callbacks without re-entering the trap', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button>first</button><button>second</button>');
    const ref = containerRef(host);
    const fallback = mount('<h1 tabindex="-1">fallback</h1>').firstElementChild as HTMLElement;
    const firstRestore = vi.fn(() => true);
    const firstReturn = vi.fn<() => FocusTarget | null>(() => null);
    const view = renderHook(
      ({
        restoreFocus,
        returnFocus,
      }: {
        restoreFocus: () => boolean;
        returnFocus: () => FocusTarget | null;
      }) => useFocusTrap({ active: true, containerRef: ref, restoreFocus, returnFocus }),
      { initialProps: { restoreFocus: firstRestore, returnFocus: firstReturn } },
    );
    const second = host.lastElementChild as HTMLButtonElement;
    second.focus();
    const latestRestore = vi.fn(() => false);
    const latestReturn = vi.fn<() => FocusTarget | null>(() => fallback);

    view.rerender({ restoreFocus: latestRestore, returnFocus: latestReturn });
    expect(document.activeElement).toBe(second);
    view.unmount();
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(firstRestore).not.toHaveBeenCalled();
    expect(firstReturn).not.toHaveBeenCalled();
    expect(latestRestore).toHaveBeenCalledTimes(1);
    expect(latestReturn).toHaveBeenCalledTimes(1);
    expect(document.activeElement).toBe(fallback);
  });

  it.each([false, true])(
    'cancels the pending restore when reopened before the next task with outside focus = %s',
    (withOutsideFocus) => {
      const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
      opener.focus();
      const focus = vi.spyOn(opener, 'focus');
      const host = mount('<button>inside</button>');
      const ref = containerRef(host);
      const view = renderHook(({ active }) => useFocusTrap({ active, containerRef: ref }), {
        initialProps: { active: true },
      });

      view.rerender({ active: false });
      if (withOutsideFocus) {
        const outside = mount('<button>outside</button>').firstElementChild as HTMLButtonElement;
        outside.focus();
      }
      view.rerender({ active: true });
      act(() => {
        vi.runOnlyPendingTimers();
      });

      expect(focus).not.toHaveBeenCalled();
      expect(document.activeElement).toBe(host.firstElementChild);

      view.unmount();
      host.remove();
      act(() => {
        vi.runOnlyPendingTimers();
      });
      expect(document.activeElement).toBe(opener);
    },
  );

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
  it('does not throw when the document goes away before the restore runs', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button>inside</button>');
    const view = renderHook(() => useFocusTrap({ active: true, containerRef: containerRef(host) }));

    view.unmount();
    vi.stubGlobal('document', undefined);
    try {
      expect(() => {
        vi.runOnlyPendingTimers();
      }).not.toThrow();
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it('cancels a pending restore when the trap reopens first', () => {
    const opener = mount('<button>open</button>').firstElementChild as HTMLButtonElement;
    opener.focus();
    const host = mount('<button id="inside">inside</button>');
    const view = renderHook(
      ({ active }) => useFocusTrap({ active, containerRef: containerRef(host) }),
      { initialProps: { active: true } },
    );
    const focus = vi.spyOn(opener, 'focus');

    view.rerender({ active: false });
    view.rerender({ active: true });
    act(() => {
      vi.runOnlyPendingTimers();
    });

    expect(focus).not.toHaveBeenCalled();
    expect(document.activeElement?.id).toBe('inside');
  });
});
