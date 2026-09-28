# @baukit/auth-node

## 0.5.0

### Minor Changes

- 98f44a8: Add `parseApiOrigin(value, { allowLoopbackHttp?, label? })` and `ApiOriginError` to the package root. The parser trims the value, accepts one trailing slash, and returns the URL's origin. It rejects credentials, a path, a query, or a fragment, and it allows plain HTTP only on a loopback host when `allowLoopbackHttp` is set. Errors carry a `reason` of `invalid_url`, `insecure_scheme`, or `not_an_origin` and never include the value. The device flow now uses the same scheme and loopback check.

  The generated MCP template now always depends on `@baukit/auth-node` and validates its API URL with `parseApiOrigin`.

  No breaking changes to the package. The CLI doctor now requires `@baukit/auth-node` in every generated `mcp/package.json`.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- 38e3201: Add a Node OIDC device-flow client with S256 PKCE, bounded requests, refresh rotation, and an atomic locked profile cache.
- Release the coordinated baukit 0.3.0 train.
