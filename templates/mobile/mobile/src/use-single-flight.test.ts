import { describe, expect, it, jest } from '@jest/globals';

import { createSingleFlight } from './use-single-flight';

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((next) => {
    resolve = next;
  });
  return { promise, resolve };
}

describe('useSingleFlight behavior', () => {
  it('runs one mutation for two activations in the same tick', async () => {
    const runSingleFlight = createSingleFlight();
    const pending = deferred<string>();
    const operation = jest.fn(() => pending.promise);

    const first = runSingleFlight(operation);
    const duplicate = runSingleFlight(operation);

    expect(operation).toHaveBeenCalledTimes(1);
    await expect(duplicate).resolves.toBeUndefined();
    pending.resolve('saved');
    await expect(first).resolves.toBe('saved');
  });

  it('releases the lock after failure', async () => {
    const runSingleFlight = createSingleFlight();
    const operation = jest
      .fn<() => Promise<string>>()
      .mockRejectedValueOnce(new Error('failed'))
      .mockResolvedValueOnce('retried');

    await expect(runSingleFlight(operation)).rejects.toThrow('failed');
    await expect(runSingleFlight(operation)).resolves.toBe('retried');
  });
});
