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
