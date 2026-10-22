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

`refreshLeewaySeconds` defaults to 30 seconds; the refresh window is the smaller of that value and half the token lifetime, measured as `expiresAt - issuedAt`.

Stored tokens record `issuedAt` and `expiresAt` in epoch milliseconds; tokens without `issuedAt` are discarded as corrupt state.

Terminal refresh rejection (`invalid_grant`, `invalid_token`, or HTTP 400/401) clears tokens, emits `subscribeSessionExpired()`, and resolves to `undefined`. Transient network/provider failures preserve the session and reject with a sanitized `OidcError` whose `retryable` property is `true`.

`subscribeSessionExpired()` reports `refresh_rejected`, `refresh_unavailable`, or `profile_erased`. On a backend 401 with code `profile_erased`, call `clearSession({ reason: 'profile_erased' })`. It clears the local session and emits the reason once per call. Plain `clearSession()` emits nothing. OIDC persists a force-login marker under `storageKeyPrefix`, so the next `login()` requests `prompt=login` even after a reload. A failed or cancelled login keeps the marker; a successful sign-in removes it. Fresh authentication gives the provider a new `auth_time` for profile re-onboarding.

The browser marker uses localStorage key `<storageKeyPrefix>:force-login` and is removed only after a successful callback token exchange stores the new session.

Display `safeAuthErrorMessage(error)` at the UI boundary. It returns only library-owned allowlisted messages and never provider descriptions, response bodies, authorization codes, or token content.

## Silent tokens for several products

`SilentTokenBroker` gets access tokens for several configured OIDC products without
leaving the current page. Each product can use its own issuer, public client ID,
redirect URI, scopes and API audience. The broker uses a hidden iframe with
`prompt=none` and S256 PKCE. It defaults to `openid` and always includes it.

```ts
import { SilentTokenBroker } from '@baukit/auth-web';

const broker = new SilentTokenBroker(
  [
    {
      id: 'documents',
      issuer: 'https://identity.example.com/realms/documents',
      clientId: 'portal-documents',
      redirectUri: `${window.location.origin}/token-callback.html`,
      scopes: ['openid', 'documents:read'],
      audience: 'documents-api',
    },
  ],
  { accountBinding: { claim: 'account_id', value: verifiedSessionSubject } },
);

const result = await broker.getToken('documents');
if (result.status === 'ok') {
  // Send result.accessToken only to the configured product API.
}

// Call directly from a click handler. The popup opens synchronously.
signInButton.onclick = () => {
  void broker.login('documents').then(renderTokenResult);
};

// After a product API rejects the token:
broker.invalidate('documents');
// Allow a deliberate silent retry after a remembered failure:
broker.resetFailure('documents');
// Before logout, session expiry or switching accounts:
broker.dispose();
```

Create one broker per authenticated session and share it across callers.
`accountBinding.claim` names a signed access token claim that must equal
`accountBinding.value`, taken from the active verified session. A provider must
protect this claim against user edits. The broker also requires matching ID and
access token subjects. Both tokens must be JWTs signed with RS256. The access
token must have an `azp` claim equal to the configured client ID. The broker
verifies signatures through the issuer's JWKS, token expiry, issuer, audience and
ID token nonce. Discovery endpoints must share the configured issuer's origin.
HTTPS is required except for HTTP loopback addresses used during development.

Optional per-product `authorizationParams` pass provider options such as an identity
provider hint. They cannot replace the broker's OIDC parameters. The broker has no
default provider hint. `getToken(productId, audience?)` accepts only the product's
configured audience when the optional audience is supplied.

`SilentTokenResult` has these outcomes:

| Status             | Fields and next step                                                                                     |
| ------------------ | -------------------------------------------------------------------------------------------------------- |
| `ok`               | `accessToken`, `expiresAt` in epoch milliseconds and `productId`                                         |
| `login_required`   | `productId` and `reason`: `login_required`, `interaction_required`, `interaction_cancelled` or `timeout` |
| `consent_required` | `productId`; offer explicit login                                                                        |
| `account_mismatch` | `productId`; the verified account or authorized client did not match                                     |
| `error`            | `productId` and a sanitized `SilentTokenError` with a library-owned `code`                               |

Every failed result stays remembered until `resetFailure(productId)` or a
successful `login(productId)`. `invalidate()` clears only the cached token.
Repeated requests for one product share a promise, including calls to `login()`
while authorization is already in flight. Call `login()` directly from a user
gesture after the pending flow finishes. Popup blocking returns an `error` result
with code `popup_blocked`, while `getToken()` keeps any earlier remembered
failure. Closing the popup returns `login_required` with reason
`interaction_cancelled` and lets the next click start a new login. The broker
checks for closure every 250 milliseconds, including during discovery and token
exchange. Custom environments can expose the live popup state through
`SilentAuthorizationWindow.closed`. Silent flows time out after 15 seconds;
interactive flows time out after three minutes. Provider descriptions and
response bodies never appear in results. Invalid configuration, unknown
products, a different requested audience and calls after disposal throw
synchronously. Constructing a broker outside a browser without an explicit
environment throws `SilentTokenError` with code `unsupported_environment`.

Tokens, failures, state, nonce and PKCE verifiers remain in memory. The broker
never reads or writes browser storage and discards refresh tokens. It renews by
starting another code flow with a 30-second expiry margin capped at half the
remaining lifetime. Expiry uses the earlier of the signed access token expiry and
the token response lifetime measured at receipt. `dispose()` aborts pending work,
closes its frames or popups and clears the cache. Pending calls resolve to an
`error` result with code `disposed`, even if the injected environment ignores
abort signals. A disposed broker cannot be reused.

Host a dedicated callback page at each configured redirect URI on the application's
origin. Its only script should call the helper below. It strips the query from
history and posts the callback URL to its same-origin parent or opener. The broker
accepts messages only from its own frame or popup on that origin, then checks the
callback path, state, issuer and code before token exchange. Providers must return
the authorization response `iss` parameter.

```ts
// Bundle as the only script on /token-callback.html.
import { completeSilentTokenCallback } from '@baukit/auth-web';

completeSilentTokenCallback();
```

`CALLBACK_MESSAGE` is also exported from `@baukit/auth-web`. Its value is
`"baukit-silent-token-callback"`. When generating an unbundled callback page,
import this constant in the code that generates the page and inject its value as
the message's `type`. Post `{ type: CALLBACK_MESSAGE, url: callbackUrl }` to the
same-origin parent or opener with `location.origin` as the target origin. Capture
`location.href` before stripping the callback query with `history.replaceState`.

Keep the callback page free of application startup and third-party assets. Set a
`no-referrer` policy and a content security policy that permits only its script.
Providers must permit framing and public-client token endpoint CORS. Third-party
cookie restrictions can prevent silent authorization; offer the explicit login
button when the broker returns a remembered failure. The broker never redirects
the main page or changes cookie policy.

## Boundaries

The package handles the OIDC authorization-code flow and the tokens it produces. It renders no UI,
depends on no framework or router, and makes no authorization decision: what a signed-in user is
allowed to do is the product's question, and the server's.

`OidcClient` token storage uses browser storage under `storageKeyPrefix`. That is a deliberate tradeoff rather
than an oversight, and it is why `safeAuthErrorMessage` exists: provider bodies, authorization
codes, and token contents never reach a message the UI can render or a logger can capture.

`@baukit/auth-native` is the same contract for React Native and Expo.

## Provider adapters

`@baukit/auth-web/clerk` exports `ClerkWebClient`, using the optional
`@clerk/clerk-js` peer. Supply a publishable key and redirect URI. An optional
JWT template can add a REST audience. The adapter uses Clerk's hosted sign-in,
refresh and sign-out without exposing its SDK to product components.

`@baukit/auth-web/workos` exports `WorkOsWebClient`, using the optional
`@workos-inc/authkit-js` peer. Supply the public AuthKit client ID and redirect
URI. Network refresh failures retain the session; terminal rejection emits the
same session-expired event as OIDC. Both adapters implement `hasSession`,
`handleCallback`, `login`, `accessToken`, `clearSession`, `logout` and
`subscribeSessionExpired`.

Clerk and browser WorkOS emit `profile_erased` and block local token access, but
do not persist a force-login marker or force fresh provider authentication. Their
SDK adapters do not expose that control through the current sign-in contract.
Configure a JWT template with `auth_time` for re-onboarding and require fresh
authentication in the provider integration before admitting an erased subject.
