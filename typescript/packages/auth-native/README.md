# `@baukit/auth-native`

Provider-neutral native OIDC authorization-code client with S256 PKCE, standard discovery, secure-storage and browser-flow ports, refresh rotation, and local-first sign-out. The core has no React, router, or product UI dependency.

```ts
import { createExpoOidcClient } from '@baukit/auth-native/expo';
import * as AuthSession from 'expo-auth-session';

const auth = createExpoOidcClient({
  issuer: 'https://identity.example.com/tenant',
  clientId: 'product-mobile',
  redirectUri: AuthSession.makeRedirectUri({ scheme: 'product', path: 'oauth' }),
  offlineAccess: true,
});

await auth.initialize();
const result = await auth.signIn();
if (result.status === 'cancelled') {
  // Restore focus or announce cancellation. This is not an authentication error.
}
```

The issuer is resolved only through `/.well-known/openid-configuration`; provider-specific paths are never manufactured. A successful authorization-code exchange is followed by an authenticated UserInfo request. Its non-empty `sub` is the authoritative subject stored in the immutable session. The package does not trust an unverified, locally decoded ID-token claim as identity.

`session()` exposes the subject, access token, optional refresh and ID tokens, and absolute expiry. Refresh responses retain the previous refresh token or ID token when the provider omits either. When a session inside the configured refresh window has no refresh token, `accessToken()` clears it and returns `undefined`; callers should present sign-in again.

`offlineAccess` deliberately defaults to `false`; set it to `true` only when the provider is configured to issue refresh tokens for `offline_access`. `accessToken()` shares one refresh across concurrent callers. Pass `{ forceRefresh: true }` after a 401 to bypass the proactive expiry window while still joining any refresh already in flight.

Terminal refresh rejection (`invalid_grant`, `invalid_token`, or HTTP 400/401) clears the session and resolves to `undefined`. Subscribe with `subscribeSessionExpired()` to stop schedulers and move UI to signed-out state. Network, malformed-response, rate-limit, and provider 5xx failures preserve the stored session and reject with a sanitized `OidcError` whose `retryable` property is `true`.

`signOut()` deletes the local session before provider interaction. It then attempts the discovered end-session endpoint. A missing, cancelled, or failing provider logout persists a fail-safe flag so the next `signIn()` includes `prompt=login`; that flag is removed only after provider logout or a later successful sign-in. Corrupt secure-storage state is deleted and treated as signed out.

Use `safeAuthErrorMessage(error)` at UI boundaries. Errors contain only allowlisted library codes/messages and optional HTTP status numbers. Provider bodies, authorization codes, tokens, and adapter exception messages are never copied into errors or logs.

The default Expo entry point uses `expo-auth-session`, `expo-secure-store`, and `expo-web-browser`, which are peer dependencies. For deterministic tests or another native stack, construct `NativeOidcClient` with your own `SecureStoragePort`, `BrowserFlowPort`, `fetch`, and clock.

Universal Expo products can pass a `storage` port to
`createExpoOidcEnvironment` or `createExpoOidcClient`. This supports a web
localStorage adapter or a product-owned compatibility/migration wrapper while
retaining the standard Expo browser flow.
