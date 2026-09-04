# Evidence for item 20: Node device authentication

## Source product files

- `tiefgang/mcp/src/auth.ts` and `tiefgang/mcp/test/auth.test.ts`
- `leitbild/mcp/src/auth.ts` and `leitbild/mcp/test/auth.test.ts`
- `eigenruhe/mcp/src/auth.ts` and `eigenruhe/mcp/test/auth.test.ts`
- `redemut/packages/mcp-server/src/login.ts`, `token-cache.ts`, and `transports/http.ts`

## Observed repeated glue and failures

Tiefgang, Leitbild, and Eigenruhe repeat discovery, device authorization, PKCE, polling, refresh, JWT decoding, and atomic cache writes. Their response bodies are unbounded, refresh has no process or file lock, and cache reads can follow symlinks. Redemut separates its cache but has the same response bounds and locking gaps. Its browser fallback also couples protocol and presentation.

## Baukit owner

`@baukit/auth-node` owns OIDC device-flow mechanics and the local token cache. It does not own CLI copy, product scopes, provider selection, or API URLs.

## Public types and errors

The public API is `DeviceFlowClient`, `NodeTokenCache`, `CachedTokenProfile`, callback and policy types, `AuthNodeError`, and `AuthNodeErrorCode`. Error messages are fixed and do not include tokens or provider bodies.

## Product-owned inputs

Products supply issuer, client ID, scopes, audience, cache namespace and path, profile, fetch, clock, sleep, abort signal, environment-token source, and presentation callbacks.

## Required cases

- Concurrency: one refresh promise per process and one adjacent cache lock across processes.
- Failure: bounded bodies, request and login timeouts, abort, denial, expiry, corruption, unsafe modes, symlinks, and interrupted replacement.
- Privacy: no logging; decoded JWT claims are unverified display hints only.
- Cleanup: temporary files and locks are removed, logout removes only the selected profile, and a failed replacement keeps the old cache.

## Supported runtimes

Node 24 or later. POSIX systems enforce `0600` cache files and a `0700` cache directory. Windows uses ACLs supplied by the host and skips POSIX mode checks.

## Product adoption change

Follow-up product pull requests should replace and delete `tiefgang/mcp/src/auth.ts`, `leitbild/mcp/src/auth.ts`, and `eigenruhe/mcp/src/auth.ts`. At least two must adopt the published package before item 20 is marked complete.
