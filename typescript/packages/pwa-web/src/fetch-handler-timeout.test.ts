import { afterEach, describe, expect, it, vi } from 'vitest';

import { CACHE_STRATEGIES, type CacheStrategy } from './cache-strategy.js';
import {
  createFetchHandler,
  type FetchHandlerPorts,
  type FetchLikeRequest,
} from './fetch-handler.js';

const APP_ORIGIN = 'https://app.example.com';
const REQUEST: FetchLikeRequest = { url: `${APP_ORIGIN}/plans`, mode: 'navigate' };

interface TestResponse {
  readonly body: string;
}

function pending<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (e: unknown) => void;
} {
  let resolve: (value: T) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function ports(
  network: Promise<TestResponse>,
  cached: TestResponse | undefined,
): {
  ports: FetchHandlerPorts<FetchLikeRequest, TestResponse>;
  puts: { strategy: CacheStrategy; body: string }[];
} {
  const puts: { strategy: CacheStrategy; body: string }[] = [];
  return {
    puts,
    ports: {
      fetch: () => network,
      matchCache: () => Promise.resolve(cached),
      putCache: (strategy, _request, response) => {
        puts.push({ strategy, body: response.body });
        return Promise.resolve();
      },
      isCacheable: () => true,
      cloneResponse: (response) => ({ ...response }),
    },
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe('networkFirst network timeout', () => {
  it('serves the cache after the default 4 s and still stores the late response', async () => {
    vi.useFakeTimers();
    const network = pending<TestResponse>();
    const harness = ports(network.promise, { body: 'cached' });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports: harness.ports });

    let settled = false;
    const result = handle(REQUEST).then((response) => {
      settled = true;
      return response;
    });
    await vi.advanceTimersByTimeAsync(3_999);
    expect(settled).toBe(false);
    await vi.advanceTimersByTimeAsync(1);
    await expect(result).resolves.toEqual({ body: 'cached' });

    network.resolve({ body: 'late' });
    await vi.advanceTimersByTimeAsync(0);
    expect(harness.puts).toEqual([{ strategy: CACHE_STRATEGIES.networkFirst, body: 'late' }]);
  });

  it('keeps waiting for the network when nothing is cached', async () => {
    vi.useFakeTimers();
    const network = pending<TestResponse>();
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      ports: ports(network.promise, undefined).ports,
      networkTimeoutMs: 100,
    });

    const result = handle(REQUEST);
    await vi.advanceTimersByTimeAsync(100);
    network.resolve({ body: 'slow network' });
    await expect(result).resolves.toEqual({ body: 'slow network' });
  });

  it('reports a late network failure after serving the cache', async () => {
    vi.useFakeTimers();
    const network = pending<TestResponse>();
    const onRevalidateError = vi.fn();
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      ports: ports(network.promise, { body: 'cached' }).ports,
      networkTimeoutMs: 100,
      onRevalidateError,
    });

    const result = handle(REQUEST);
    await vi.advanceTimersByTimeAsync(100);
    await expect(result).resolves.toEqual({ body: 'cached' });
    const failure = new Error('offline');
    network.reject(failure);
    await vi.advanceTimersByTimeAsync(0);
    expect(onRevalidateError).toHaveBeenCalledWith(failure);
  });

  it('waits for the network when the timeout is disabled', async () => {
    vi.useFakeTimers();
    const network = pending<TestResponse>();
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      ports: ports(network.promise, { body: 'cached' }).ports,
      networkTimeoutMs: false,
    });

    const result = handle(REQUEST);
    await vi.advanceTimersByTimeAsync(60_000);
    network.resolve({ body: 'network' });
    await expect(result).resolves.toEqual({ body: 'network' });
  });

  it('rejects an invalid timeout', () => {
    expect(() =>
      createFetchHandler({
        appOrigin: APP_ORIGIN,
        ports: ports(Promise.resolve({ body: 'x' }), undefined).ports,
        networkTimeoutMs: -1,
      }),
    ).toThrow(RangeError);
  });
});
