// @vitest-environment jsdom
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';

import { useSingleFlight } from './use-single-flight.js';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((onResolve) => {
    resolve = onResolve;
  });
  return { promise, resolve };
}

afterEach(() => {
  cleanup();
});

describe('useSingleFlight', () => {
  it('runs an operation and returns its value', async () => {
    const view = renderHook(() => useSingleFlight());

    await expect(view.result.current(() => Promise.resolve('saved'))).resolves.toBe('saved');
  });

  it('rejects a second call while the first is still running', async () => {
    const view = renderHook(() => useSingleFlight());
    const gate = deferred<string>();
    let secondRan = false;

    const first = view.result.current(() => gate.promise);
    const second = await view.result.current(() => {
      secondRan = true;
      return Promise.resolve('ignored');
    });

    expect(second).toBeUndefined();
    expect(secondRan).toBe(false);

    await act(async () => {
      gate.resolve('first');
      await first;
    });
    await expect(first).resolves.toBe('first');
  });

  it('accepts the next call once the previous one settles', async () => {
    const view = renderHook(() => useSingleFlight());

    await view.result.current(() => Promise.resolve('one'));

    await expect(view.result.current(() => Promise.resolve('two'))).resolves.toBe('two');
  });

  it('releases the lock after a failed operation', async () => {
    const view = renderHook(() => useSingleFlight());
    const failure = new Error('write rejected');

    await expect(view.result.current(() => Promise.reject(failure))).rejects.toBe(failure);

    await expect(view.result.current(() => Promise.resolve('recovered'))).resolves.toBe(
      'recovered',
    );
  });

  it('keeps the same runner across re-renders', () => {
    const view = renderHook(() => useSingleFlight());
    const first = view.result.current;

    view.rerender();

    expect(view.result.current).toBe(first);
  });
});
