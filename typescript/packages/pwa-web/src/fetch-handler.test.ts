import { describe, expect, it, vi } from 'vitest';

import { CACHE_STRATEGIES, type CacheStrategy } from './cache-strategy.js';
import {
  createFetchHandler,
  type FetchHandlerPorts,
  type FetchLikeRequest,
} from './fetch-handler.js';

const APP_ORIGIN = 'https://app.example.com';

interface TestResponse {
  readonly body: string;
  readonly cacheable?: boolean;
}

interface Harness {
  readonly ports: FetchHandlerPorts<FetchLikeRequest, TestResponse>;
  readonly puts: { strategy: CacheStrategy; url: string; body: string }[];
  readonly fetchCalls: { url: string; noStore: boolean }[];
  readonly matchCalls: string[];
}

function harness(overrides: {
  cached?: TestResponse;
  cachedByUrl?: Readonly<Record<string, TestResponse>>;
  network?: TestResponse | (() => Promise<TestResponse>);
}): Harness {
  const puts: Harness['puts'] = [];
  const fetchCalls: Harness['fetchCalls'] = [];
  const matchCalls: string[] = [];
  const network = overrides.network ?? { body: 'network' };

  return {
    puts,
    fetchCalls,
    matchCalls,
    ports: {
      fetch: async (request, init) => {
        fetchCalls.push({ url: request.url, noStore: init?.cache === 'no-store' });
        return typeof network === 'function' ? network() : network;
      },
      matchCache: (request) => {
        const url = typeof request === 'string' ? request : request.url;
        matchCalls.push(url);
        return Promise.resolve(overrides.cachedByUrl?.[url] ?? overrides.cached);
      },
      putCache: (strategy, request, response) => {
        puts.push({ strategy, url: request.url, body: response.body });
        return Promise.resolve();
      },
      isCacheable: (response) => response.cacheable !== false,
      cloneResponse: (response) => ({ ...response }),
    },
  };
}

describe('createFetchHandler', () => {
  it('bypasses the cache for a never-cached path', async () => {
    const { ports, fetchCalls, puts } = harness({ cached: { body: 'stale' } });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      neverCachedPathPrefixes: ['/api/v1/sync'],
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/api/v1/sync/pull` })).resolves.toEqual({
      body: 'network',
    });
    expect(fetchCalls).toEqual([{ url: `${APP_ORIGIN}/api/v1/sync/pull`, noStore: true }]);
    expect(puts).toEqual([]);
  });

  it('serves a cache-first hit without touching the network', async () => {
    const { ports, fetchCalls } = harness({ cached: { body: 'cached-icon' } });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({
      body: 'cached-icon',
    });
    expect(fetchCalls).toEqual([]);
  });

  it('fetches and stores a cache-first miss', async () => {
    const { ports, puts } = harness({});
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({ body: 'network' });
    expect(puts).toEqual([
      { strategy: CACHE_STRATEGIES.cacheFirst, url: `${APP_ORIGIN}/icon.png`, body: 'network' },
    ]);
  });

  it('does not store an uncacheable cache-first response', async () => {
    const { ports, puts } = harness({ network: { body: 'error', cacheable: false } });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({
      body: 'error',
      cacheable: false,
    });
    expect(puts).toEqual([]);
  });

  it('stores a successful network-first response', async () => {
    const { ports, puts } = harness({});
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' })).resolves.toEqual({
      body: 'network',
    });
    expect(puts).toEqual([
      { strategy: CACHE_STRATEGIES.networkFirst, url: `${APP_ORIGIN}/plans`, body: 'network' },
    ]);
  });

  it('falls back to the cache when a network-first fetch fails', async () => {
    const { ports } = harness({
      cached: { body: 'shell' },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' })).resolves.toEqual({
      body: 'shell',
    });
  });

  it('uses the configured app shell after a navigation request misses the cache', async () => {
    const { ports, matchCalls } = harness({
      cachedByUrl: { '/index.html': { body: 'app-shell' } },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      navigationFallback: '/index.html',
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/plans/active`, mode: 'navigate' })).resolves.toEqual({
      body: 'app-shell',
    });
    expect(matchCalls).toEqual([`${APP_ORIGIN}/plans/active`, '/index.html']);
  });

  it('runs an overridden navigation strategy before the fallback lookup', async () => {
    const { ports, matchCalls } = harness({
      cachedByUrl: { '/index.html': { body: 'app-shell' } },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      defaultStrategy: CACHE_STRATEGIES.cacheFirst,
      navigationFallback: '/index.html',
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' })).resolves.toEqual({
      body: 'app-shell',
    });
    expect(matchCalls).toEqual([`${APP_ORIGIN}/plans`, '/index.html']);
  });

  it('does not use the navigation fallback for an API route', async () => {
    const { ports, matchCalls } = harness({
      cachedByUrl: { '/index.html': { body: 'app-shell' } },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      navigationFallback: '/index.html',
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/api/v1/me`, mode: 'navigate' })).rejects.toThrow(
      'offline',
    );
    expect(matchCalls).toEqual([`${APP_ORIGIN}/api/v1/me`]);
  });

  it('does not use the navigation fallback for a non-navigation request', async () => {
    const { ports, matchCalls } = harness({
      cachedByUrl: { '/index.html': { body: 'app-shell' } },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      navigationFallback: '/index.html',
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/plans` })).rejects.toThrow('offline');
    expect(matchCalls).toEqual([`${APP_ORIGIN}/plans`]);
  });

  it('keeps navigation misses unchanged when no fallback is configured', async () => {
    const { ports, matchCalls } = harness({
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' })).rejects.toThrow(
      'offline',
    );
    expect(matchCalls).toEqual([`${APP_ORIGIN}/plans`]);
  });

  it('rethrows when a network-first fetch fails with nothing cached', async () => {
    const { ports } = harness({ network: () => Promise.reject(new Error('offline')) });
    const handle = createFetchHandler({ appOrigin: APP_ORIGIN, ports });

    await expect(handle({ url: `${APP_ORIGIN}/api/v1/me` })).rejects.toThrow('offline');
  });

  it('returns the cached response and revalidates in the background', async () => {
    const { ports, puts } = harness({ cached: { body: 'stale' } });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      staticStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({ body: 'stale' });
    await vi.waitFor(() => {
      expect(puts).toEqual([
        {
          strategy: CACHE_STRATEGIES.staleWhileRevalidate,
          url: `${APP_ORIGIN}/icon.png`,
          body: 'network',
        },
      ]);
    });
  });

  it('awaits the network on a stale-while-revalidate miss', async () => {
    const { ports, puts } = harness({});
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      staticStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({ body: 'network' });
    expect(puts).toHaveLength(1);
  });

  it('reports a background revalidation failure instead of rejecting', async () => {
    const onRevalidateError = vi.fn();
    const { ports } = harness({
      cached: { body: 'stale' },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      staticStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      ports,
      onRevalidateError,
    });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({ body: 'stale' });
    await vi.waitFor(() => {
      expect(onRevalidateError).toHaveBeenCalledWith(new Error('offline'));
    });
  });

  it('swallows a background revalidation failure with no reporter', async () => {
    const { ports } = harness({
      cached: { body: 'stale' },
      network: () => Promise.reject(new Error('offline')),
    });
    const handle = createFetchHandler({
      appOrigin: APP_ORIGIN,
      staticStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      ports,
    });

    await expect(handle({ url: `${APP_ORIGIN}/icon.png` })).resolves.toEqual({ body: 'stale' });
  });
});
