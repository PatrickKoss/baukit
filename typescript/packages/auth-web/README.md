# `@baukit/auth-web`

Browser sign-in for baukit products. `BrowserSessionClient` runs the OIDC authorization-code
redirect with S256 PKCE, hands the code to the product backend's `/auth/session` routes
(`baukit_auth::browser_session::router`), and keeps the access token in memory only. The backend
redeems the code, keeps the refresh token in its session table and sets an HttpOnly
`__Host-` cookie. Design and threat model: [ADR 0006](../../../docs/adr/0006-browser-token-handling.md).

```ts
import { BrowserSessionClient } from '@baukit/auth-web';

const auth = new BrowserSessionClient({
  issuer: 'https://identity.example.com/realms/product/',
  clientId: 'product-web',
  redirectUri: `${window.location.origin}/`,
  apiBaseUrl: 'https://api.example.com',
});

await auth.handleCallback(); // after the IdP redirect; false on any other URL
await auth.restore(); // on load: one refresh when this browser signed in before
const accessToken = await auth.getAccessToken(); // send as `Authorization: Bearer`
```

| Call                            | Request                                                                  | What the client does                                                                                                                                                        |
| ------------------------------- | ------------------------------------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `handleCallback()`              | `POST /auth/session/exchange` with `{ code, codeVerifier, redirectUri }` | Checks state and issuer, strips the callback parameters, stores the token in memory and the account hint. Repeated calls share one promise.                                 |
| `getAccessToken()`, `restore()` | `POST /auth/session/refresh`                                             | Returns the memory token until `expiresAt - min(refreshLeewaySeconds, lifetime / 2)`, then refreshes. `{ forceRefresh: true }` refreshes now, for example after an API 401. |
| `logout()`                      | `POST /auth/session/logout`                                              | Signs out every tab first, then navigates to the `endSessionUrl` the backend returns (http or https only). Resolves `false` on `204`.                                       |

Session calls are the only calls with `credentials: 'include'`. Each sends `X-Baukit-Session: 1`
(`BROWSER_SESSION_HEADER`) and `cache: 'no-store'`. API calls should keep the default
`same-origin` credentials and send the bearer token.

### What lives where

- The access token, its expiry and subject live in the client object. Reload means one refresh.
- `localStorage[<storageKeyPrefix>:account]` holds `{ subject, signedInAt }` and nothing else.
  `account()` reads it, so an offline reload can open the right local profile. `hasSession()` is
  true while a token is in memory or the hint exists.
- `sessionStorage[<storageKeyPrefix>:transaction]` holds the PKCE `state` and `verifier` between
  `login()` and the callback.
- `localStorage[<storageKeyPrefix>:force-login]` is `1` after `clearSession({ reason:
'profile_erased' })`, so the next `login()` sends `prompt=login`. A successful exchange removes it.

Nothing token-shaped goes to localStorage, sessionStorage, IndexedDB or the Cache API, and the
package never reads a `<prefix>:tokens` key.

### Tabs

A refresh runs under the Web Lock `<storageKeyPrefix>:refresh`. The winner posts
`{ accessToken, expiresAt, subject }` on `BroadcastChannel('<storageKeyPrefix>:session')`; a tab
that was waiting for the lock takes that token instead of refreshing. Sign-out, expiry and
`clearSession()` are broadcast too, and other tabs drop their token and emit
`onSessionExpired`. Without Web Locks each tab refreshes on its own and the backend's 10-second
grace window answers the loser. Call `dispose()` to close the channel.

### Failures and offline

| Refresh outcome                               | Result                                                                                          |
| --------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `401` (`session_expired`)                     | Clears the token and account hint in every tab, emits `refresh_rejected`, resolves `undefined`. |
| Network error, `429`, `5xx`                   | Rejects with `OidcError('refresh_failed', { retryable: true })`. The account hint stays.        |
| Any other status, such as `403 csrf_rejected` | Rejects with a non-retryable `refresh_failed`. The account hint stays.                          |

Only a 401 signs the browser out. Queue sync and API calls while `getAccessToken()` rejects with a
retryable error and open local data from the account hint.

`onSessionExpired()` (alias `subscribeSessionExpired()`) reports `refresh_rejected`,
`profile_erased`, or `signed_out` when another tab logged out or cleared the session. This tab's
own `logout()` and plain `clearSession()` emit nothing. `accessToken()` is an alias of
`getAccessToken()`, so the client fits the same provider contract as the Clerk and WorkOS
adapters.

Display `safeAuthErrorMessage(error)` at the UI boundary. It returns only library-owned
allowlisted messages and never provider descriptions, response bodies, authorization codes, or
token content.

### `OidcClient`

`OidcClient` is the redirect half on its own: `discover()`, `login()`, `completeCallback()`
(returns `{ code, codeVerifier, redirectUri }` or `undefined`), `requireFreshLogin()`,
`confirmFreshLogin()` and `clearTransaction()`. It never redeems a code and holds no token. The
issuer is normalized and resolved only through `/.well-known/openid-configuration`. Pass a unique
`storageKeyPrefix` when more than one client for the same issuer and client ID shares an origin.
`offlineAccess` (default `false`) adds `offline_access`; `openid` is always included.

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

The package handles the browser side of the OIDC authorization-code flow. It renders no UI,
depends on no framework or router, and makes no authorization decision: what a signed-in user is
allowed to do is the product's question, and the server's. The refresh token never reaches the
browser; the product backend holds it.

`@baukit/auth-native` is the same contract for React Native and Expo.

## Provider adapters

`@baukit/auth-web/clerk` exports `ClerkWebClient`, using the optional
`@clerk/clerk-js` peer. Supply a publishable key and redirect URI. An optional
JWT template can add a REST audience. The adapter uses Clerk's hosted sign-in,
refresh and sign-out without exposing its SDK to product components.

`@baukit/auth-web/workos` exports `WorkOsWebClient`, using the optional
`@workos-inc/authkit-js` peer. Supply the public AuthKit client ID and redirect
URI. Network refresh failures retain the session; terminal rejection emits the
same session-expired event as `BrowserSessionClient`. Both adapters implement `hasSession`,
`handleCallback`, `login`, `accessToken`, `clearSession`, `logout` and
`subscribeSessionExpired`.

Clerk and browser WorkOS emit `profile_erased` and block local token access, but
do not persist a force-login marker or force fresh provider authentication. Their
SDK adapters do not expose that control through the current sign-in contract.
Configure a JWT template with `auth_time` for re-onboarding and require fresh
authentication in the provider integration before admitting an erased subject.
