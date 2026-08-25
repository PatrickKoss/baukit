// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

const platform = { OS: 'web' as string };

vi.mock('react-native', () => ({
  get Platform() {
    return platform;
  },
}));

import { useEnterToNext } from './use-enter-to-next.js';

/** A real input stands in for the TextInput that React Native Web renders. */
function input(id: string): HTMLInputElement {
  const node = document.createElement('input');
  node.id = id;
  document.body.appendChild(node);
  return node;
}

afterEach(() => {
  cleanup();
  document.body.innerHTML = '';
  platform.OS = 'web';
});

describe('useEnterToNext on web', () => {
  it('moves focus to the next field and submits from the last one', () => {
    const onSubmit = vi.fn();
    const view = renderHook(() => useEnterToNext(2, onSubmit));
    const first = view.result.current.registerField(0);
    const second = view.result.current.registerField(1);
    first.ref(input('first') as never);
    second.ref(input('second') as never);

    expect(first.returnKeyType).toBe('next');
    expect(second.returnKeyType).toBe('done');

    act(() => {
      first.onSubmitEditing?.({} as never);
    });
    expect(document.activeElement?.id).toBe('second');
    expect(onSubmit).not.toHaveBeenCalled();

    act(() => {
      second.onSubmitEditing?.({} as never);
    });
    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('skips a multiline field and leaves its Enter key alone', () => {
    const onSubmit = vi.fn();
    const view = renderHook(() => useEnterToNext(3, onSubmit));
    const first = view.result.current.registerField(0);
    const multiline = view.result.current.registerField(1, { multiline: true });
    const third = view.result.current.registerField(2);
    first.ref(input('first') as never);
    multiline.ref(input('multiline') as never);
    third.ref(input('third') as never);

    expect(multiline.returnKeyType).toBeUndefined();
    expect(multiline.onSubmitEditing).toBeUndefined();
    expect(third.returnKeyType).toBe('done');

    act(() => {
      first.onSubmitEditing?.({} as never);
    });
    expect(document.activeElement?.id).toBe('third');
  });

  it('submits when no later field has mounted yet', () => {
    const onSubmit = vi.fn();
    const view = renderHook(() => useEnterToNext(2, onSubmit));
    view.result.current.registerField(1);

    act(() => {
      view.result.current.registerField(0).onSubmitEditing?.({} as never);
    });

    expect(onSubmit).toHaveBeenCalledTimes(1);
  });

  it('marks the last single-line field done when the form ends with a multiline field', () => {
    const view = renderHook(() => useEnterToNext(2, vi.fn()));
    view.result.current.registerField(1, { multiline: true });

    expect(view.result.current.registerField(0).returnKeyType).toBe('done');
  });

  it('re-registering a field as single-line puts it back in the chain', () => {
    const view = renderHook(() => useEnterToNext(2, vi.fn()));
    view.result.current.registerField(1, { multiline: true });
    view.result.current.registerField(1);

    expect(view.result.current.registerField(0).returnKeyType).toBe('next');
  });

  it('exposes submit directly', () => {
    const onSubmit = vi.fn();
    const view = renderHook(() => useEnterToNext(1, onSubmit));

    act(() => {
      view.result.current.submit();
    });

    expect(onSubmit).toHaveBeenCalledTimes(1);
  });
});

describe('useEnterToNext on native', () => {
  it.each(['ios', 'android'])('leaves the platform keyboard behavior alone on %s', (os) => {
    platform.OS = os;
    const view = renderHook(() => useEnterToNext(2, vi.fn()));

    const props = view.result.current.registerField(0);

    expect(props.returnKeyType).toBeUndefined();
    expect(props.onSubmitEditing).toBeUndefined();
  });
});
