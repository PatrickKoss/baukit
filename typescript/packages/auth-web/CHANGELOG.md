# @baukit/auth-web

## Unreleased

## 0.20.0

### Minor Changes

- `BrowserSessionClient.login(options)` and `OidcClient.login(options)` accept `stateDecoration` (segments of ASCII letters and digits placed before the random state nonce, built with `appearanceStateDecoration`) and `entry: 'register'` (sends `prompt=create`). Invalid segments or colors throw before any navigation.

### Patch Changes

- `OidcClient.completeCallback` (and `BrowserSessionClient.handleCallback`) only treat the configured redirect URI (origin and path) as an OIDC callback. App URLs that carry their own `code` or `state` parameters, such as a suite-link return URL, no longer fail with `callback_state_mismatch`.

### Minor Changes

- Release the coordinated baukit 0.20.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.20.0

## 0.19.2

### Patch Changes

- Release the coordinated baukit 0.19.2 train.
- Updated dependencies
  - @baukit/type-guards@0.19.2

## 0.19.1

### Patch Changes

- Release the coordinated baukit 0.19.1 train.
- Updated dependencies
  - @baukit/type-guards@0.19.1

## 0.19.0

### Major Changes

- Add `BrowserSessionClient` (ADR 0006). It redeems the authorization code through the product
  backend's `/auth/session/{exchange,refresh,logout}` routes with `credentials: 'include'` and
  `X-Baukit-Session: 1`, keeps the access token in memory only, refreshes once on load, coordinates
  refresh across tabs with Web Locks and `BroadcastChannel`, broadcasts sign-out, and keeps only
  `{ subject, signedInAt }` in `localStorage[<prefix>:account]`. A network error, 429 or 5xx on
  refresh is retryable and keeps the account hint; only a 401 signs out.
- `OidcClient` no longer stores, refreshes or reads tokens and no longer touches
  `<prefix>:tokens`. It keeps `discover()` and `login()` and gains `completeCallback()`,
  `requireFreshLogin()`, `confirmFreshLogin()` and `clearTransaction()`. `accessToken()`,
  `handleCallback()`, `hasSession()`, `logout()`, `clearSession()`, `subscribeSessionExpired()`,
  `postLogoutRedirectUri` and `refreshLeewaySeconds` moved to `BrowserSessionClient`. No
  compatibility mode.
- `SessionExpiredReason` drops `refresh_unavailable` and adds `signed_out`. New `OidcErrorCode`
  `logout_failed`.

### Minor Changes

- Release the coordinated baukit 0.19.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/type-guards@0.19.0

## 0.18.3

### Patch Changes

- Release the coordinated baukit 0.18.3 train.

## 0.18.2

### Patch Changes

- Release the coordinated baukit 0.18.2 train.

## 0.18.1

### Patch Changes

- Release the coordinated baukit 0.18.1 train.

## 0.18.0

### Minor Changes

- Release the coordinated baukit 0.18.0 train.

## 0.17.1

### Patch Changes

- Release the coordinated baukit 0.17.1 train.

## 0.17.0

### Minor Changes

- Release the coordinated baukit 0.17.0 train.

## 0.16.0

### Minor Changes

- Release the coordinated baukit 0.16.0 train.

## 0.15.0

### Minor Changes

- Release the coordinated baukit 0.15.0 train.

## 0.14.1

### Patch Changes

- Release the coordinated baukit 0.14.1 train.

## 0.14.0

### Minor Changes

- Release the coordinated baukit 0.14.0 train.

## 0.13.0

### Minor Changes

- Release the coordinated baukit 0.13.0 train.

## 0.12.2

### Patch Changes

- Release the coordinated baukit 0.12.2 train.

## 0.12.1

### Patch Changes

- Release the coordinated baukit 0.12.1 train.

## 0.12.0

### Minor Changes

- Release the coordinated baukit 0.12.0 train.

## 0.11.3

### Patch Changes

- Release the coordinated baukit 0.11.3 train.

## 0.11.2

### Patch Changes

- Release the coordinated baukit 0.11.2 train.

## 0.11.1

### Patch Changes

- Release the coordinated baukit 0.11.1 train.

## 0.11.0

- Add `SilentTokenBroker` for silent multi-product OIDC authorization with in-memory tokens, remembered failures, account binding and explicit popup login. A blocked popup preserves earlier failures, and closing a popup returns `login_required` with reason `interaction_cancelled`.
- Add `completeSilentTokenCallback` and the `CALLBACK_MESSAGE` constant for dedicated callback pages.
- Add optional `SilentAuthorizationWindow.closed` for custom popup environments and the `unsupported_environment` error code for construction outside a browser without an explicit environment.

### Minor Changes

- Release the coordinated baukit 0.11.0 train.
- 7fdf020: Add `SilentTokenBroker` for silent authorization against multiple OIDC realms with PKCE, verified account claims, in-memory token caches and explicit popup login. Blocked popups preserve earlier remembered failures; closed popups return `login_required` with reason `interaction_cancelled`. Custom environments can expose popup closure through optional `SilentAuthorizationWindow.closed`. Construction outside a browser without an explicit environment throws `SilentTokenError` with code `unsupported_environment`. Add `completeSilentTokenCallback` and the `CALLBACK_MESSAGE` constant for same-origin callback pages.

## 0.10.32

### Patch Changes

- Release the coordinated baukit 0.10.32 train.

## 0.10.31

### Patch Changes

- Release the coordinated baukit 0.10.31 train.

## 0.10.30

### Patch Changes

- Release the coordinated baukit 0.10.30 train.

## 0.10.29

### Patch Changes

- Release the coordinated baukit 0.10.29 train.

## 0.10.28

### Patch Changes

- Release the coordinated baukit 0.10.28 train.

## 0.10.27

### Patch Changes

- Release the coordinated baukit 0.10.27 train.

## 0.10.26

### Patch Changes

- Release the coordinated baukit 0.10.26 train.

## 0.10.25

- Cap the refresh window at half the access token lifetime so short-lived tokens do not refresh on every call.
- Store `issuedAt` in epoch milliseconds and discard persisted tokens without it.

### Patch Changes

- Release the coordinated baukit 0.10.25 train.

## 0.10.24

### Patch Changes

- Release the coordinated baukit 0.10.24 train.

## 0.10.23

### Patch Changes

- Release the coordinated baukit 0.10.23 train.

## 0.10.22

### Patch Changes

- Release the coordinated baukit 0.10.22 train.

## 0.10.21

### Patch Changes

- Release the coordinated baukit 0.10.21 train.

## 0.10.20

### Patch Changes

- Release the coordinated baukit 0.10.20 train.

## 0.10.19

- Add `profile_erased` to session-expired reasons and accept it in `clearSession`. Plain clearing stays silent.
- Persist fresh OIDC authentication after erasure. The next sign-in requests `prompt=login` until a successful sign-in clears the marker.
- Isolate session-expired listener errors in the provider adapters. Document Clerk and browser WorkOS re-onboarding requirements.

### Minor Changes

- 43021fe: Add the `profile_erased` session-expired reason to `clearSession`. OIDC persists a force-login marker and requests `prompt=login` until a successful sign-in. Clerk and browser WorkOS emit the reason; their provider integrations need fresh authentication and JWT `auth_time` for re-onboarding.

### Patch Changes

- Release the coordinated baukit 0.10.19 train.

## 0.10.18

### Patch Changes

- Release the coordinated baukit 0.10.18 train.

## 0.10.17

### Patch Changes

- Release the coordinated baukit 0.10.17 train.

## 0.10.16

### Patch Changes

- Release the coordinated baukit 0.10.16 train.

## 0.10.15

### Patch Changes

- Release the coordinated baukit 0.10.15 train.

## 0.10.14

### Patch Changes

- Release the coordinated baukit 0.10.14 train.

## 0.10.13

### Patch Changes

- Release the coordinated baukit 0.10.13 train.

## 0.10.12

### Patch Changes

- Release the coordinated baukit 0.10.12 train.

## 0.10.11

### Patch Changes

- Release the coordinated baukit 0.10.11 train.

## 0.10.10

### Patch Changes

- Release the coordinated baukit 0.10.10 train.

## 0.10.9

### Patch Changes

- Release the coordinated baukit 0.10.9 train.

## 0.10.8

### Patch Changes

- Release the coordinated baukit 0.10.8 train.

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

## 0.9.0

- Pass optional API audience and resource parameters in OIDC authorization requests.
- Add Clerk and WorkOS SDK adapters behind the browser authentication contract.

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

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/auth-web`.
