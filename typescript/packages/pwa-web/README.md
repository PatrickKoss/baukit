# `@baukit/pwa-web`

`@baukit/pwa-web` decides which caching strategy a PWA service worker should apply to a request.
The decision is a pure function over the request's method, URL, mode, and destination. Nothing in
this package reads `self`, `caches`, `clients`, or `location`, so every branch is unit-testable in
plain Node.

Products own the cache names, precache list, install and activate handlers, and offline fallback
copy. This package owns the classification and the strategy execution around whatever cache the
product supplies.

## Classifying a request

```ts
import { CACHE_STRATEGIES, decideCacheStrategy } from '@baukit/pwa-web';

decideCacheStrategy(
  { url: '/api/v1/sync/pull?since=12', method: 'GET' },
  { appOrigin: 'https://app.example.com', neverCachedPathPrefixes: ['/api/v1/sync'] },
);
// 'network-only'
```

The rules run in this order:

1. A path under any `neverCachedPathPrefixes` entry is `network-only`. The list is empty by
   default, so no endpoint is special until a product names one. A sync endpoint belongs here:
   replaying a stale delta corrupts the local database.
2. Any non-GET method is `network-only`.
3. A path under any `apiPathPrefixes` entry (default `['/api']`) gets `apiStrategy`, default
   `network-first`.
4. A navigation (`mode === 'navigate'`) gets `defaultStrategy`, default `network-first`. When
   `navigationFallback` is set, `createFetchHandler` looks up that app-shell path if the selected
   strategy fails and the original request is not cached.
5. A same-origin request whose destination, file extension, or path prefix marks it static gets
   `staticStrategy`, default `cache-first`.
6. Everything else gets `defaultStrategy`.

Prefix matching is segment-aware: `/api` matches `/api` and `/api/v1/me` but not `/apiary`.
Extension matching reads the last path segment only, so `/v1.2/plans` is not a static asset.
Cross-origin assets never reach `cache-first`, because a service worker should not fill its cache
with another origin's responses.

### Options

| Option                    | Default                                          | Purpose                                                         |
| ------------------------- | ------------------------------------------------ | --------------------------------------------------------------- |
| `appOrigin`               | required                                         | Base for relative URLs and the same-origin test                 |
| `neverCachedPathPrefixes` | `[]`                                             | Paths that must always hit the network, such as a sync endpoint |
| `apiPathPrefixes`         | `['/api']`                                       | Paths treated as API reads                                      |
| `navigationFallback`      | unset                                            | Cached app-shell path used after an offline navigation miss     |
| `staticPathPrefixes`      | `[]`                                             | Extensionless static paths, such as `/_expo/static/`            |
| `staticDestinations`      | audio, font, image, script, style, video, worker | `Request.destination` values that mean static                   |
| `staticExtensions`        | common web asset extensions                      | File extensions that mean static                                |
| `apiStrategy`             | `network-first`                                  | Strategy for API reads                                          |
| `staticStrategy`          | `cache-first`                                    | Strategy for static assets                                      |
| `defaultStrategy`         | `network-first`                                  | Strategy for everything else                                    |

`createCacheStrategyDecider(options)` binds the options once and returns a function of the
request, which is what an install-time asset scan wants.

`navigationFallback` applies only to same-origin GET requests with `mode: 'navigate'`. API routes
and paths in `neverCachedPathPrefixes` never use it, even if a caller supplies `mode: 'navigate'`.
The selected `defaultStrategy` still runs first. The handler checks the fallback path only after
that strategy cannot return a network response or a cached response for the requested URL. With no
fallback configured, navigation misses keep the previous behavior and reject with the network
error. The product must precache the fallback path.

## Running the strategy

`createFetchHandler` executes the chosen strategy through ports the product provides. It never
constructs a `Cache` or calls the global `fetch` itself, so the same handler runs under Vitest
with plain objects.

```js
// public/sw.js
import { createFetchHandler } from '/pwa-web.js';

const STATIC_CACHE = 'app-static-v1';
const SHELL_CACHE = 'app-shell-v1';

const handleFetch = createFetchHandler({
  appOrigin: self.location.origin,
  neverCachedPathPrefixes: ['/api/v1/sync'],
  navigationFallback: '/index.html',
  staticPathPrefixes: ['/_expo/static/'],
  ports: {
    fetch: (request, init) => fetch(request, init),
    matchCache: (request) => caches.match(request),
    putCache: async (strategy, request, response) => {
      const name = strategy === 'cache-first' ? STATIC_CACHE : SHELL_CACHE;
      const cache = await caches.open(name);
      await cache.put(request, response);
    },
    isCacheable: (response) => response.ok && response.type !== 'error',
    cloneResponse: (response) => response.clone(),
  },
  onRevalidateError: () => {
    // A background refresh failure must not surface to the page.
  },
});

self.addEventListener('fetch', (event) => {
  event.respondWith(handleFetch(event.request));
});
```

Behavior per strategy:

- `network-only` fetches with `cache: 'no-store'` and never writes to the cache.
- `cache-first` returns a hit, otherwise fetches and stores a cacheable response.
- `network-first` fetches and stores a cacheable response, and falls back to the cache when the
  fetch throws. If the request is an eligible navigation and its own cache entry is absent,
  `navigationFallback` supplies one final cache lookup. With nothing cached it rethrows.
- `stale-while-revalidate` returns the cached response and refreshes in the background. On a
  miss it awaits the network. A background failure goes to `onRevalidateError`.

`putCache` receives the strategy that produced the response, which is how the snippet above
routes static assets and shell responses into different caches.

## Use the worker artifact

`@baukit/pwa-web/worker` resolves to a bundled classic script. It has no imports and assigns the
public API to `globalThis.BaukitPwa`. A build script can copy it without parsing or rewriting
package source:

```js
import { copyFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const source = fileURLToPath(import.meta.resolve('@baukit/pwa-web/worker'));
await copyFile(source, 'public/baukit-pwa-worker.js');
```

Load the copy before product worker code:

```js
// public/sw.js
importScripts('/baukit-pwa-worker.js');

const handleFetch = globalThis.BaukitPwa.createFetchHandler({
  // Product cache policy and ports go here.
});
```

Register `sw.js` as a normal classic service worker:

```js
if ('serviceWorker' in navigator) {
  await navigator.serviceWorker.register('/sw.js');
}
```

The IIFE format is deliberate. Vite copies its `public/` directory into `dist/`, and Expo web does
the same during static export. Both paths can copy this artifact byte for byte. Neither path needs
bare module imports in a browser, module-worker registration, or package-specific bundler setup.
The packed npm tarball test loads the artifact with no `window`, `document`, or `self` global.

### Migrate a source-rewriting build

Replace code that reads `dist/cache-strategy.js`, removes imports or exports, and concatenates files
with one copy from `@baukit/pwa-web/worker`. Keep the product-owned `sw.js` and its cache policy.
Imports from `@baukit/pwa-web` and CommonJS `require('@baukit/pwa-web')` remain supported.

## Cache cleanup

Products decide which exact cache names belong to an old version or identity. `cleanupCaches`
provides the list-and-delete operation and returns counts without returning or logging cache names.
This activate handler removes old application cache versions while preserving the current version
and caches owned by other code:

```js
const CURRENT_APP_CACHE = 'notes-app-v4';

self.addEventListener('activate', (event) => {
  event.waitUntil(
    globalThis.BaukitPwa.cleanupCaches({
      ports: {
        listCacheNames: () => caches.keys(),
        deleteCache: (name) => caches.delete(name),
      },
      shouldDelete: (name) => name.startsWith('notes-app-') && name !== CURRENT_APP_CACHE,
    }),
  );
});
```

Private caches need a product-owned, opaque partition key. On an identity change, delete the exact
previous partition before exposing the new identity's data. On logout, delete every private cache
for the outgoing partition. Do not put access tokens, email addresses, or raw identity-provider
subjects in cache names.

```ts
import { cleanupCaches } from '@baukit/pwa-web';

await cleanupCaches({
  ports: {
    listCacheNames: () => caches.keys(),
    deleteCache: (name) => caches.delete(name),
  },
  shouldDelete: (name) => name === `notes-private-${outgoingPartitionKey}`,
});
```

`cleanupCaches` selects all names before starting deletes, then runs the deletes together. It counts
a cache as deleted only when `CacheStorage.delete` resolves to `true`. A list, predicate, or delete
failure rejects the operation. Some deletes may already have completed when another delete rejects,
so retry cleanup before completing logout or an identity switch.

## What this package does not do

It does not register the service worker, precache an app shell, choose cache names, define routes,
create a web manifest or icons, prompt for updates, choose an offline route, or derive identity
partition keys. Those depend on the product's build output, privacy model, and copy. Baukit's CLI
still requires the web capability for PWA use. Publishing this artifact does not add PWA generation
to Expo web projects.
