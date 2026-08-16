// @vitest-environment jsdom

import { cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { useSingleFlight } from './use-single-flight';

afterEach(cleanup);

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe('useSingleFlight', () => {
  it('runs one mutation for two activations in the same tick', async () => {
    const result = renderHook(() => useSingleFlight());
    const pending = deferred<string>();
    const operation = vi.fn(() => pending.promise);

    const first = result.result.current(operation);
    const duplicate = result.result.current(operation);

    expect(operation).toHaveBeenCalledTimes(1);
    await expect(duplicate).resolves.toBeUndefined();
    pending.resolve('saved');
    await expect(first).resolves.toBe('saved');
  });

  it('releases the lock after failure', async () => {
    const result = renderHook(() => useSingleFlight());
    const operation = vi
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error('failed'))
      .mockResolvedValueOnce('retried');

    await expect(result.result.current(operation)).rejects.toThrow('failed');
    await expect(result.result.current(operation)).resolves.toBe('retried');
  });
});
