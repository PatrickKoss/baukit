# PWA worker artifact evidence

## Source product files

- `/home/patrick/projects/eigenruhe/mobile/scripts/build-sw.mjs`
- `/home/patrick/projects/eigenruhe/mobile/public/sw-strategy.js`
- `/home/patrick/projects/eigenruhe/mobile/public/sw.js`
- `/home/patrick/projects/eigenruhe/mobile/src/pwa/register-service-worker.ts`
- `/home/patrick/projects/eigenruhe/mobile/package.json`

## Observed failure or repeated glue

Eigenruhe resolves the package's ESM entry, reads two compiled files, strips imports, exports, and
source-map comments with regular expressions, then concatenates the results into a classic worker
global. A compiler output change can break that build without changing the package API.

## Baukit owner

`@baukit/pwa-web` owns the cache strategy code, its worker-safe distribution format, and generic
list-and-delete cache cleanup.

## Public types and errors

`@baukit/pwa-web/worker` is a classic-script IIFE that assigns the existing API and `cleanupCaches`
to `globalThis.BaukitPwa`. The ESM and CommonJS entries add `cleanupCaches`, `CacheCleanupOptions`,
`CacheCleanupPorts`, and `CacheCleanupResult`. The helper preserves failures from product ports and
defines no package-specific error.

## Product-owned inputs

Products own `sw.js`, registration timing, cache names and versions, routes, manifest metadata,
icons, precache lists, offline routes and responses, cacheability policy, and opaque identity
partition keys.

## Cases

- Concurrency: cleanup selects names once and deletes matches concurrently. A retry is safe when
  product predicates select only stale or outgoing caches.
- Failure: list, predicate, and delete failures reject. Concurrent deletes can partially complete.
- Privacy: result values contain counts, not cache names. Products must not use tokens, email
  addresses, or raw provider subjects as partition keys.
- Cleanup: activation removes old versions. Identity switch and logout remove the outgoing private
  partition before the product exposes another identity or an unauthenticated state.

## Supported runtimes and artifact decision

The package targets ES2022 service workers. A classic-script IIFE was tested as a byte-for-byte
`public/` copy for the generated Vite web layout and Eigenruhe's Expo SDK 57 web layout. Both build
paths copy public assets into their static output, so neither needs a worker bundler. Node 24 loads
the built artifact in a worker-like context without DOM page globals. Expo PWA generation remains a
separate CLI decision, and `capabilities.pwa` still requires `capabilities.web`.

## Product adoption change

Eigenruhe can replace the regex transforms and reads of `cache-strategy.js` and `fetch-handler.js`
in `mobile/scripts/build-sw.mjs` with one copy from `@baukit/pwa-web/worker`. Its product-owned worker
assembly and `mobile/public/sw.js` remain. `mobile/public/sw-strategy.js` becomes a direct copy of
the published artifact rather than rewritten package source.
