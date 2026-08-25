import { describe, expect, it } from 'vitest';

import {
  CACHE_STRATEGIES,
  createCacheStrategyDecider,
  decideCacheStrategy,
} from './cache-strategy.js';

const APP_ORIGIN = 'https://app.example.com';
const OPTIONS = { appOrigin: APP_ORIGIN };

describe('decideCacheStrategy', () => {
  it('treats non-GET methods as network-only', () => {
    for (const method of ['POST', 'put', 'DELETE', 'PATCH']) {
      expect(decideCacheStrategy({ url: `${APP_ORIGIN}/plans`, method }, OPTIONS)).toBe(
        CACHE_STRATEGIES.networkOnly,
      );
    }
  });

  it('defaults to GET when the method is omitted', () => {
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/me` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('caches no API path by default beyond network-first', () => {
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/me` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('does not treat a path that merely shares a prefix as an API path', () => {
    expect(
      decideCacheStrategy({ url: `${APP_ORIGIN}/apiary/photo.png`, destination: 'image' }, OPTIONS),
    ).toBe(CACHE_STRATEGIES.cacheFirst);
  });

  it('never caches a configured sync path and its subpaths', () => {
    const options = { appOrigin: APP_ORIGIN, neverCachedPathPrefixes: ['/api/v1/sync'] };
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/sync` }, options)).toBe(
      CACHE_STRATEGIES.networkOnly,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/sync/pull?since=12` }, options)).toBe(
      CACHE_STRATEGIES.networkOnly,
    );
    expect(
      decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/sync/push`, method: 'POST' }, options),
    ).toBe(CACHE_STRATEGIES.networkOnly);
  });

  it('leaves the sync path unconfigured by default', () => {
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/sync` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('accepts a trailing slash on a configured prefix', () => {
    const options = { appOrigin: APP_ORIGIN, neverCachedPathPrefixes: ['/api/v1/sync/'] };
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/sync` }, options)).toBe(
      CACHE_STRATEGIES.networkOnly,
    );
  });

  it('ignores an empty prefix entry', () => {
    const options = { appOrigin: APP_ORIGIN, neverCachedPathPrefixes: [''] };
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/plans` }, options)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('uses network-first for navigations even when the path looks static', () => {
    expect(
      decideCacheStrategy({ url: `${APP_ORIGIN}/report.json`, mode: 'navigate' }, OPTIONS),
    ).toBe(CACHE_STRATEGIES.networkFirst);
  });

  it('keeps navigation classification network-first when a fallback is configured', () => {
    const options = { appOrigin: APP_ORIGIN, navigationFallback: '/index.html' };
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' }, options)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('uses cache-first for same-origin static destinations', () => {
    for (const destination of ['audio', 'font', 'image', 'script', 'style', 'video', 'worker']) {
      expect(decideCacheStrategy({ url: `${APP_ORIGIN}/asset`, destination }, OPTIONS)).toBe(
        CACHE_STRATEGIES.cacheFirst,
      );
    }
  });

  it('uses cache-first for same-origin static extensions without a destination', () => {
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/icon-512.png` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/manifest.webmanifest` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/app.a1b2c3.JS` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/bundle.js?v=3` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
  });

  it('does not read an extension out of a directory segment or a dotfile', () => {
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/v1.2/plans` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/.png` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/plans` }, OPTIONS)).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });

  it('uses cache-first for a configured static path prefix with no extension', () => {
    const options = { appOrigin: APP_ORIGIN, staticPathPrefixes: ['/_expo/static/'] };
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/_expo/static/js/web/app` }, options)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
  });

  it('keeps cross-origin static assets on the default strategy', () => {
    expect(
      decideCacheStrategy(
        { url: 'https://cdn.example.com/logo.png', destination: 'image' },
        OPTIONS,
      ),
    ).toBe(CACHE_STRATEGIES.networkFirst);
  });

  it('resolves a relative URL against the app origin', () => {
    expect(decideCacheStrategy({ url: '/icon-192.png', destination: 'image' }, OPTIONS)).toBe(
      CACHE_STRATEGIES.cacheFirst,
    );
  });

  it('honors overridden strategies and destination lists', () => {
    const options = {
      appOrigin: APP_ORIGIN,
      apiPathPrefixes: ['/graphql'],
      apiStrategy: CACHE_STRATEGIES.networkOnly,
      staticStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      defaultStrategy: CACHE_STRATEGIES.staleWhileRevalidate,
      staticDestinations: ['image'],
      staticExtensions: ['png'],
    } as const;

    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/graphql` }, options)).toBe(
      CACHE_STRATEGIES.networkOnly,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/api/v1/me` }, options)).toBe(
      CACHE_STRATEGIES.staleWhileRevalidate,
    );
    expect(decideCacheStrategy({ url: `${APP_ORIGIN}/logo.png` }, options)).toBe(
      CACHE_STRATEGIES.staleWhileRevalidate,
    );
    expect(
      decideCacheStrategy({ url: `${APP_ORIGIN}/app.js`, destination: 'script' }, options),
    ).toBe(CACHE_STRATEGIES.staleWhileRevalidate);
  });
});

describe('createCacheStrategyDecider', () => {
  it('binds options once and classifies many requests', () => {
    const decide = createCacheStrategyDecider({
      appOrigin: APP_ORIGIN,
      neverCachedPathPrefixes: ['/api/v1/sync'],
    });

    expect(decide({ url: `${APP_ORIGIN}/api/v1/sync/pull` })).toBe(CACHE_STRATEGIES.networkOnly);
    expect(decide({ url: `${APP_ORIGIN}/icon.png` })).toBe(CACHE_STRATEGIES.cacheFirst);
    expect(decide({ url: `${APP_ORIGIN}/plans`, mode: 'navigate' })).toBe(
      CACHE_STRATEGIES.networkFirst,
    );
  });
});
