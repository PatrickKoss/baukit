# @baukit/auth-node

## Unreleased

- Move shipped notes out of Unreleased into their release sections.

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- bfc7aac: `allowKeycloakWebOrigin(stack, origin)` also adds `<origin>/*` to the web client's `post.logout.redirect.uris` attribute, keeping the other attributes and any existing entries, so a signed-out test lands back on the app. A `+` entry already covers the redirect URIs and stays as it is. The `/keycloak-testing` subpath stays ESM only; the README now states that CommonJS consumers load it through Node 24 `require(esm)` and type-check it with `"module": "node20"` or `"nodenext"`, and the packed-exports test loads it with `require`.
- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- 306a46f: Add the Node-only `@baukit/auth-node/keycloak-testing` subpath for end-to-end tests against a disposable development Keycloak. `keycloakStack(defaults, environment)` reads `E2E_KEYCLOAK_URL`, `E2E_KEYCLOAK_REALM`, `E2E_KEYCLOAK_ADMIN_USERNAME`, `E2E_KEYCLOAK_ADMIN_PASSWORD`, and `E2E_KEYCLOAK_WEB_CLIENT_ID` over the product's defaults. `createKeycloakTestUser(stack, user?, options?)` creates a verified user, random `e2e-<uuid>` by default, and returns its username, password, email, and subject. `revokeKeycloakUserSessions(stack, subject)` ends a user's sessions. `allowKeycloakWebOrigin(stack, origin)` adds an origin to the web client's redirect URIs and web origins when it lacks them. `signInWithKeycloak(page, user, stack, { timeoutMs })` fills the login form by its `#username`, `#password`, and `#kc-login` IDs. Admin calls use `fetch` with a 30-second default timeout and take an injected `fetch`; errors name the step and HTTP status, never a response body. The page parameter is a structural type that a Playwright `Page` satisfies, so the package still has no runtime or peer dependencies.

  No breaking changes to existing exports.

- Release the coordinated baukit 0.5.1 train.

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
