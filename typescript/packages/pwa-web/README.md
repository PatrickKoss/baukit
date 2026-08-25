# @baukit/pwa-web

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

## Build a service worker without a bundler

The package exports ESM for `import` and CommonJS for `require`. Build scripts can load either
format directly:

```js
const { decideCacheStrategy } = require('@baukit/pwa-web');
```

A service worker copied to `public/` cannot resolve `node_modules`. The following
`scripts/build-sw.mjs` reads the installed ESM files, removes their module syntax, and writes one
classic script. It exposes the package through `globalThis.BaukitPwa`, so a classic worker can use
`importScripts('/sw-strategy.js')`.

```js
import { readFile, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const outputPath = process.argv[2] ?? 'public/sw-strategy.js';
const entryPath = fileURLToPath(import.meta.resolve('@baukit/pwa-web'));
const distDir = dirname(entryPath);

async function readDistFile(name) {
  const source = await readFile(join(distDir, name), 'utf8');
  return source
    .replace(/import\s*\{[^}]+\}\s*from\s*['"]\.\/cache-strategy\.js['"];\s*/s, '')
    .replace(/^export /gm, '')
    .replace(/^\/\/# sourceMappingURL=.*$/gm, '')
    .trim();
}

const source = `// Generated by scripts/build-sw.mjs. Do not edit.
${await readDistFile('cache-strategy.js')}
${await readDistFile('fetch-handler.js')}
globalThis.BaukitPwa = Object.freeze({
  CACHE_STRATEGIES,
  createCacheStrategyDecider,
  createFetchHandler,
  decideCacheStrategy,
});
`;

await writeFile(outputPath, source);
```

Add generation and drift checks to the product's `package.json`:

```json
{
  "scripts": {
    "build:sw": "node scripts/build-sw.mjs public/sw-strategy.js",
    "build:sw:check": "sh -c 'tmp=$(mktemp); trap \"rm -f $tmp\" EXIT; node scripts/build-sw.mjs \"$tmp\" && diff -u public/sw-strategy.js \"$tmp\"'"
  }
}
```

Run `build:sw` after updating `@baukit/pwa-web`. Commit the generated file and run
`build:sw:check` in CI. The diff fails when the installed package and committed worker differ.

## What this package does not do

It does not register the service worker, precache an app shell, version caches, prompt for
updates, or supply an offline page. Those depend on the product's build output and copy.
