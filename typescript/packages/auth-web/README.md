# `@baukit/auth-web`

Framework-neutral browser OIDC authorization-code client with S256 PKCE, standard provider discovery, refresh tokens, and callback deduplication for repeated UI effects.

```ts
import { OidcClient } from '@baukit/auth-web';

const auth = new OidcClient({
  issuer: 'https://identity.example.com/realms/product/',
  clientId: 'product-web',
  redirectUri: `${window.location.origin}/auth/callback`,
  scopes: ['openid', 'profile', 'email'],
  offlineAccess: true,
});

await auth.login();
await auth.handleCallback();
const accessToken = await auth.accessToken();
```

The issuer is normalized and resolved only through `/.well-known/openid-configuration`; the client never assumes provider-specific authorization, token, or logout paths. `handleCallback()` returns the same promise when called repeatedly on one client, making a one-time PKCE exchange safe under React Strict Mode without depending on React.

Pass a unique `storageKeyPrefix` when more than one client for the same issuer/client ID shares an origin. Set `offlineAccess` to add `offline_access` without duplicating it. `openid` is always included because it distinguishes OIDC from plain OAuth.

`offlineAccess` defaults to `false`; enable it deliberately only for providers configured to issue refresh tokens for that scope. Concurrent `accessToken()` calls share one refresh. `accessToken({ forceRefresh: true })` bypasses the proactive window after a 401 while joining any refresh already in flight. Refresh rotation retains the previous refresh or ID token when a response omits a replacement, and expiry is anchored to local token receipt time.

Terminal refresh rejection (`invalid_grant`, `invalid_token`, or HTTP 400/401) clears tokens, emits `subscribeSessionExpired()`, and resolves to `undefined`. Transient network/provider failures preserve the session and reject with a sanitized `OidcError` whose `retryable` property is `true`.

Display `safeAuthErrorMessage(error)` at the UI boundary. It returns only library-owned allowlisted messages and never provider descriptions, response bodies, authorization codes, or token content.
