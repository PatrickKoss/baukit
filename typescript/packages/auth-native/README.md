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

Session and force-login keys use dot separators. The default prefix encodes the
issuer and client ID with SecureStore-safe characters. A custom `storageKeyPrefix`
must be non-empty and contain only ASCII letters, digits, `.`, `-` and `_`.
Construction rejects invalid prefixes before accessing storage. No key rewrite
adapter is needed. Old keys are not migrated.

Universal Expo products can pass a `storage` port to
`createExpoOidcEnvironment` or `createExpoOidcClient`. This supports a web
localStorage adapter or a product-owned compatibility/migration wrapper while
retaining the standard Expo browser flow.

## Themed login pages

A login theme can match the app's appearance when the app tells it the theme
through the OAuth `state`. Pass the segments to `signIn` and give the Expo flow
an entropy source:

```ts
import { appearanceStateDecoration } from '@baukit/auth-native';
import { createExpoOidcClient } from '@baukit/auth-native/expo';
import * as Crypto from 'expo-crypto';

const auth = createExpoOidcClient(config, {
  randomBytes: (size) => Crypto.getRandomBytesAsync(size),
});

await auth.signIn({ stateDecoration: appearanceStateDecoration({ mode: 'dark' }) });
```

The state becomes `ap1.d.<64 hex characters>`, or
`ap1.d.<PRIMARY>.<SECONDARY>.<nonce>` when you pass both `#RRGGBB` colors.
`decoratedAuthorizationState` needs at least 32 random bytes and accepts only
ASCII letters and digits per segment. The nonce keeps the state unguessable, and
the client still compares the returned state with the one it sent.

Decoration never blocks sign-in. Without `randomBytes`, or when the entropy
source throws or a segment is malformed, the flow uses AuthSession's own state
and the login page shows its default theme. `createExpoBrowserFlow` is the same
flow on its own, for products that build their `NativeOidcEnvironment` by hand.
Other `BrowserFlowPort` implementations may ignore `stateDecoration`.

The state travels in the authorization URL, so the provider and its access logs
see it. Put appearance hints there, never user data. The Keycloak theme from the
`baukit` CLI's OIDC template decodes the `ap1` format.

## Boundaries

The package owns the native OIDC flow, session storage, and refresh. It ships no screens, no
navigation, and no authorization logic; what a signed-in user may do is decided by the product and
enforced by the server.

The core has no React, router, or product UI dependency, and the Expo entry point is a separate
subpath. Construct `NativeOidcClient` with your own `SecureStoragePort`, `BrowserFlowPort`, `fetch`,
and clock when you are on another native stack or want a deterministic test.

`@baukit/auth-web` is the same contract for browsers.
