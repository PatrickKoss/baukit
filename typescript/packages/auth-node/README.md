# @baukit/auth-node

`@baukit/auth-node` is the Node 24 OIDC device-flow package for CLI and MCP clients. It handles discovery, RFC 8628 polling, S256 PKCE, refresh rotation, and a local profile cache. It has no runtime dependencies.

The package does not print instructions or open a browser. Supply callbacks for those actions.

## Device-flow client

```ts
import { DeviceFlowClient } from '@baukit/auth-node/device-flow';

const auth = new DeviceFlowClient(
  {
    issuer: process.env['OIDC_ISSUER'] ?? '',
    clientId: process.env['OIDC_CLIENT_ID'] ?? '',
    scopes: ['openid', 'profile', 'offline_access'],
    audience: 'notes-api',
    cache: {
      namespace: 'notes-mcp',
    },
  },
  {
    environmentToken: () => process.env['NOTES_API_TOKEN'],
  },
);

await auth.login({
  presentation: {
    showVerification: ({ verificationUri, userCode }) => {
      process.stderr.write(`Open ${verificationUri}\nEnter ${userCode}\n`);
    },
    showStatus: (status) => {
      process.stderr.write(`Login status: ${status}\n`);
    },
    openBrowser: (url) => openProductBrowser(url),
  },
});

const token = await auth.accessToken();
```

`accessToken()` checks the injected environment-token source first. It then reads the selected cache profile and refreshes near-expiry tokens. Refreshes share one promise in a process and hold an adjacent `.lock` file while reading and replacing the cache.

Pass an `AbortSignal` to `login`, `accessToken`, or `logout`. Discovery and token requests have a 15-second default timeout. A login has a 10-minute total timeout. Both limits are configurable.

## Endpoint policy

The configured issuer must match discovery metadata. Put known issuer aliases in `endpointPolicy.issuerAllowlist`. Token and device endpoints must share the issuer origin and path. Put a provider's documented endpoint origin in `endpointOriginAllowlist` when it uses a separate origin.

All issuer, device, token, and verification URLs require HTTPS. Local development can set `allowLoopbackHttp: true`; this permits only `localhost`, `127.0.0.0/8`, and `::1`.

Discovery and token bodies are limited to 64 KiB by default. Errors contain a stable code, an allowlisted message, an optional HTTP status, and no provider body. The client does not log.

## API origins

`parseApiOrigin(value, { allowLoopbackHttp, label })` checks a configured API base URL, such as an MCP server's `PRODUCT_API_URL`, with the same scheme rule as the endpoint policy. It trims whitespace, accepts one trailing slash, and returns `url.origin`.

```ts
import { parseApiOrigin } from '@baukit/auth-node';

const apiUrl = parseApiOrigin(process.env['PRODUCT_API_URL'] ?? 'http://localhost:8080', {
  allowLoopbackHttp: process.env['NODE_ENV'] !== 'production',
  label: 'PRODUCT_API_URL',
});
```

It throws `ApiOriginError`, a `TypeError`, with a `reason` of `invalid_url`, `insecure_scheme`, or `not_an_origin`. Credentials, a path, a query, or a fragment count as `not_an_origin`. `allowLoopbackHttp` defaults to false. The message names the label and the broken rule, never the value, so it is safe to log.

## Keycloak test helpers

`@baukit/auth-node/keycloak-testing` sets up users for end-to-end tests against a disposable development Keycloak. It runs on Node 24, uses the global `fetch`, and needs no Playwright dependency.

The subpath ships as ESM only, like every Baukit package. Node 24 loads it from CommonJS through `require(esm)`, so a CommonJS test project needs no dynamic `import()`. TypeScript models that only with `"module": "node20"` or `"nodenext"`; under `"node16"` it reports TS1479 on the import, because `node16` describes a Node release that could not `require` ESM.

```ts
import {
  allowKeycloakWebOrigin,
  createKeycloakTestUser,
  keycloakStack,
  signInWithKeycloak,
} from '@baukit/auth-node/keycloak-testing';

const stack = keycloakStack({
  url: 'http://localhost:8081',
  realm: 'notes',
  webClientId: 'notes-web',
});

await allowKeycloakWebOrigin(stack, 'http://localhost:5183');
const user = await createKeycloakTestUser(stack);
await page.getByRole('button', { name: 'Sign in' }).click();
await signInWithKeycloak(page, user, stack);
```

`keycloakStack(defaults, environment = process.env)` returns the product's defaults unless `E2E_KEYCLOAK_URL`, `E2E_KEYCLOAK_REALM`, `E2E_KEYCLOAK_ADMIN_USERNAME`, `E2E_KEYCLOAK_ADMIN_PASSWORD`, or `E2E_KEYCLOAK_WEB_CLIENT_ID` is set. The admin credentials default to `admin` and `admin`, and trailing slashes are removed from the URL.

`createKeycloakTestUser(stack, user?, options?)` signs in to the master realm's `admin-cli` and creates a verified, enabled user with a permanent password. Without `user`, the username is `e2e-<uuid>` and the password is random, so parallel tests never share an identity. Pass `username`, `email`, or `password` when the realm signs in by email. It returns `{ username, password, email, subject }`; `subject` is the Keycloak user ID, which becomes the `sub` claim.

`revokeKeycloakUserSessions(stack, subject)` ends every session of the user, so the app's next refresh fails. `allowKeycloakWebOrigin(stack, origin)` adds `<origin>/*` to the web client's redirect URIs and to its `post.logout.redirect.uris` attribute, and `origin` to its web origins. It keeps the client's other attributes, and does nothing when all three are present. A post-logout list that holds `+` already reuses the redirect URIs, so the helper leaves it alone. `signInWithKeycloak(page, user, stack, { timeoutMs })` waits until the page is on the Keycloak origin, fills `#username` and `#password`, and clicks `#kc-login`; those IDs do not change with the login locale. `page` is any object with Playwright's `waitForURL` and `locator`, such as a `Page` or a sign-in popup.

Every admin call takes `{ fetch, timeoutMs }`; the timeout defaults to `DEFAULT_KEYCLOAK_REQUEST_TIMEOUT_MS` (30 seconds). A failure throws an `Error` that names the step and HTTP status and never includes a response body or the admin password. Created users and added origins stay in the realm, so point the helpers only at a development realm.

## Cache contract

The JSON cache holds named profiles under one namespace. `defaultTokenCachePath(namespace)` resolves to `$XDG_CONFIG_HOME/<namespace>/tokens.json`, or `~/.config/<namespace>/tokens.json` when `XDG_CONFIG_HOME` is unset.

On POSIX systems the cache file must have mode `0600` and its immediate directory must have mode `0700`. Existing unsafe permissions fail with `cache_permission`. The cache rejects symlink path components. Writes use a new temporary file, sync it, and atomically rename it, so a write failure before rename leaves the old file intact. Windows skips POSIX mode checks and relies on host ACLs.

Use a separate namespace or profile for accounts that must not share credentials. `logout()` removes the selected profile and leaves other profiles in place.

## Display-only claims

`displayClaims()` and `decodeDisplayOnlyClaims()` decode a small allowlist of JWT fields without checking the signature. Use the result only for labels such as a `whoami` display. Never use it for authorization, storage partitions, audit identity, or analytics identity.

## Migration from product-local auth

Replace local discovery, device polling, refresh, and cache functions with one `DeviceFlowClient`. Keep these product inputs in the application:

- environment variable names and defaults;
- issuer, client ID, scopes, and audience;
- CLI text and browser-launch behavior;
- API base URL and bearer-token wiring; and
- cache namespace, path, and profile selection.

Older product caches do not have the versioned profile document used here. Sign out or remove the old file, then run the product's login command once. The package intentionally does not guess which product-owned legacy shape it received.
