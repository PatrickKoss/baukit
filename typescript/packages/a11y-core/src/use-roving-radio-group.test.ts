// @vitest-environment jsdom
import { act, cleanup, render, renderHook } from '@testing-library/react';
import { createElement } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';

vi.mock('react-native', () => ({ Platform: { OS: 'web' } }));

import {
  nextIndexFor,
  useRovingRadioGroup,
  type RovingKeyEvent,
} from './use-roving-radio-group.js';

const OPTIONS = ['low', 'medium', 'high'] as const;

function keyEvent(key: string) {
  const preventDefault = vi.fn<() => void>();
  const event: RovingKeyEvent = { nativeEvent: { key }, preventDefault };
  return { ...event, preventDefault };
}

function byId(id: string): HTMLElement {
  const element = document.querySelector<HTMLElement>(`#${id}`);
  if (element === null) throw new Error(`no element with id ${id}`);
  return element;
}

/** React Native Web hands a DOM element to a ref typed as a native View. */
function attachRef(attach: (node: never) => void, element: HTMLElement): void {
  attach(element as never);
}

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
});

describe('nextIndexFor', () => {
  it.each([
    ['ArrowDown', 1, 2],
    ['ArrowRight', 1, 2],
    ['ArrowUp', 1, 0],
    ['ArrowLeft', 1, 0],
    ['Home', 1, 0],
    ['End', 1, 3],
  ])('maps %s from index %i to %i', (key, index, expected) => {
    expect(nextIndexFor(key, index, 4)).toBe(expected);
  });

  it('wraps around both ends of the group', () => {
    expect(nextIndexFor('ArrowDown', 3, 4)).toBe(0);
    expect(nextIndexFor('ArrowUp', 0, 4)).toBe(3);
  });

  it('ignores keys that are not group movement', () => {
    expect(nextIndexFor('Tab', 1, 4)).toBeNull();
    expect(nextIndexFor('Enter', 1, 4)).toBeNull();
    expect(nextIndexFor(' ', 1, 4)).toBeNull();
  });

  it('stays on the only option in a single-option group', () => {
    expect(nextIndexFor('ArrowDown', 0, 1)).toBe(0);
    expect(nextIndexFor('End', 0, 1)).toBe(0);
  });
});

describe('useRovingRadioGroup', () => {
  it('gives the group one tab stop on the selected option', () => {
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange: vi.fn(), options: OPTIONS, value: 'medium' }),
    );

    expect(view.result.current.radioProps(0).tabIndex).toBe(-1);
    expect(view.result.current.radioProps(1).tabIndex).toBe(0);
    expect(view.result.current.radioProps(2).tabIndex).toBe(-1);
  });

  it('falls back to the first option when the value is not in the list', () => {
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange: vi.fn(), options: OPTIONS, value: 'absent' }),
    );

    expect(view.result.current.radioProps(0).tabIndex).toBe(0);
  });

  it('ignores keys that do not move the group', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange, options: OPTIONS, value: 'low' }),
    );

    const event = keyEvent('Tab');
    act(() => {
      view.result.current.radioProps(0).onKeyDown(event);
    });

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('does nothing for an empty group', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup<string>({ onChange, options: [], value: 'low' }),
    );

    const event = keyEvent('ArrowDown');
    act(() => {
      view.result.current.radioProps(0).onKeyDown(event);
    });

    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(onChange).not.toHaveBeenCalled();
  });

  it('selects without focusing when the option has not mounted', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange, options: OPTIONS, value: 'low' }),
    );

    expect(() => {
      act(() => {
        view.result.current.radioProps(0).onKeyDown(keyEvent('End'));
      });
    }).not.toThrow();
    expect(onChange).toHaveBeenCalledWith('high');
  });
});

describe('useRovingRadioGroup in a rendered group', () => {
  function Group({ value, onChange }: { value: string; onChange: (next: string) => void }) {
    const { radioProps } = useRovingRadioGroup({ onChange, options: OPTIONS, value });
    return createElement(
      'div',
      { role: 'radiogroup' },
      OPTIONS.map((option, index) => {
        const props = radioProps(index);
        return createElement(
          'button',
          {
            key: option,
            id: option,
            role: 'radio',
            'aria-checked': option === value,
            ref: props.ref as never,
            tabIndex: props.tabIndex,
            onKeyDown: props.onKeyDown as never,
          },
          option,
        );
      }),
    );
  }

  it('exposes exactly one tab stop in the DOM', () => {
    render(createElement(Group, { value: 'medium', onChange: vi.fn() }));

    const stops = Array.from(document.querySelectorAll('[role="radio"]')).filter(
      (radio) => radio.getAttribute('tabindex') === '0',
    );

    expect(stops.map((radio) => radio.id)).toEqual(['medium']);
  });

  it('moves real focus to the next option on an arrow key', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange, options: OPTIONS, value: 'low' }),
    );
    render(createElement(Group, { value: 'low', onChange }));

    // Attach the hook's refs to the rendered radios, as React Native Web does.
    OPTIONS.forEach((option, index) => {
      attachRef(view.result.current.radioProps(index).ref, byId(option));
    });
    byId('low').focus();

    act(() => {
      view.result.current.radioProps(0).onKeyDown(keyEvent('ArrowDown'));
    });

    expect(onChange).toHaveBeenCalledWith('medium');
    expect(document.activeElement?.id).toBe('medium');
  });

  it('wraps focus from the last option back to the first', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange, options: OPTIONS, value: 'high' }),
    );
    render(createElement(Group, { value: 'high', onChange }));
    OPTIONS.forEach((option, index) => {
      attachRef(view.result.current.radioProps(index).ref, byId(option));
    });
    byId('high').focus();

    act(() => {
      view.result.current.radioProps(2).onKeyDown(keyEvent('ArrowRight'));
    });

    expect(onChange).toHaveBeenCalledWith('low');
    expect(document.activeElement?.id).toBe('low');
  });

  it('jumps to the last option on End', () => {
    const onChange = vi.fn();
    const view = renderHook(() =>
      useRovingRadioGroup({ onChange, options: OPTIONS, value: 'low' }),
    );
    render(createElement(Group, { value: 'low', onChange }));
    OPTIONS.forEach((option, index) => {
      attachRef(view.result.current.radioProps(index).ref, byId(option));
    });

    act(() => {
      view.result.current.radioProps(0).onKeyDown(keyEvent('End'));
    });

    expect(onChange).toHaveBeenCalledWith('high');
    expect(document.activeElement?.id).toBe('high');
  });
});
