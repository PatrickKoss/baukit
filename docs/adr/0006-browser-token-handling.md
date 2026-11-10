# ADR 0006: Browser token handling

## Status

Accepted, 2026-11-07. Implements decision D2 of the
[quality and security plan](../quality-security-plan.md) and fixes S-01 and S-02
([E4-01, E4-02](../quality-security/E4-security.md)).

## Context

`OidcClient` in `typescript/packages/auth-web/src/index.ts` runs authorization
code with PKCE S256 in the browser, then writes access, refresh and ID tokens as
JSON to `localStorage[<prefix>:tokens]` (`writeTokens`, line 428). Refresh runs
from the browser against the IdP token endpoint (`refresh`, line 349). Any XSS or
compromised npm dependency can read the refresh token and keep the account for
the refresh token's lifetime. No baukit crate, template or chart sets a CSP.

Other relevant facts:

- The web template builds the client in `templates/web/__auth__/web/src/auth.ts`
  with `offlineAccess` on by default, so Keycloak issues offline tokens.
- Products run web and API on different origins: `VITE_API_URL` points at
  `http://localhost:<api_port>` in `templates/web/__auth__/web/.env.example`,
  schluesselbund and architecture-health-platform do the same. Different ports
  on one host are the same site, so cookies with `SameSite=Strict` still flow.
- `baukit-http` builds CORS from `HttpOptions.allowed_origins` and
  `allow_credentials` (`rust/crates/baukit-http/src/middleware.rs:166`). It sets
  no security headers.
- `baukit-auth` verifies bearer tokens under four profiles (OIDC, Clerk,
  client-bound, scope-bound; `rust/crates/baukit-auth/src/config.rs:7`) and
  already owns a PostgreSQL store with migrations (`postgres.rs`,
  `migrations/0001_baukit_auth_api_tokens.sql`).
- `baukit-credential-vault` has a versioned AEAD cipher over aws-lc-rs with key
  rotation (`rust/crates/baukit-credential-vault/src/cipher.rs`).
- `ClerkWebClient` (`auth-web/src/clerk.ts`) and `WorkOsWebClient`
  (`auth-web/src/workos.ts`) delegate sessions to the vendor SDKs.
- The silent-token broker (`auth-web/src/silent-token-broker.ts`) already keeps
  per-product access tokens in memory only, for suite sign-in.
- Native apps (`auth-native`, secure storage) and MCP clients send bearer tokens
  and never touch browser storage.
- Products are local-first PWAs. Local data lives in IndexedDB and must open
  offline, without a token.

## Decision

The browser never holds a refresh token. The product backend becomes a narrow
backend-for-frontend (BFF) for token issuance only. API calls keep using
`Authorization: Bearer`, so the verifier, MCP and native apps do not change.

### 1. Endpoints

`baukit-auth` gets an axum router, `browser_session::router(state)`, that
products mount at the reserved prefix `/auth/session`. The browser still
redirects to the IdP and receives the code on its own origin. It posts the code
and the PKCE verifier to the backend, which redeems them. The SPA keeps control
of routing and the PWA shell works unchanged.

| Method and path | Request body | Response |
|---|---|---|
| `POST /auth/session/exchange` | `{ "code", "codeVerifier", "redirectUri" }` | `200 { "accessToken", "expiresAt", "subject" }`, sets the cookie |
| `POST /auth/session/refresh` | empty | `200 { "accessToken", "expiresAt", "subject" }`, rotates the cookie; `401 { "code": "session_expired" }` clears it |
| `POST /auth/session/logout` | empty | `204`, clears the cookie |

`expiresAt` is epoch milliseconds. `subject` is the `sub` claim, so the client
can detect an account switch without an ID token. The backend keeps the ID
token server side for `id_token_hint` on logout. `redirectUri` must match an
entry in `BrowserSessionConfig.redirect_uris` after WHATWG URL serialization on
both sides, so an origin registered with or without the trailing slash works. Errors use the baukit-http
problem shape with codes `invalid_grant`, `session_expired`, `csrf_rejected`,
`idp_unavailable` (503, retryable).

The backend calls the IdP as the same OIDC client the SPA uses today. A public
client stays public; a confidential client secret is optional config for IdPs
that require one. Logout revokes the refresh token at the IdP
`revocation_endpoint` (RFC 7009) when discovery lists it, deletes the session
row, clears the cookie, and returns `{ "endSessionUrl" }` in a `200` instead of
`204` when the IdP has an `end_session_endpoint`, for the browser to navigate to.

Rejected: backend handles the redirect (`/auth/session/callback`). It moves the
callback off the SPA, needs a state cookie before login, and breaks the
existing `handleCallback` and service-worker navigation fallback for no gain,
since the code is single-use and bound to the PKCE verifier only the tab holds.

### 2. Cookie

```
Set-Cookie: __Host-<product>-session=<session_id>.<secret>; Path=/; Secure; HttpOnly;
            SameSite=Strict; Max-Age=<idle_timeout>
```

- `__Host-` forbids `Domain` and needs `Path=/` and `Secure`. It stops a sibling
  subdomain from planting or shadowing the cookie. The cost is that the cookie
  also rides on API requests; the API ignores it everywhere except
  `/auth/session/*`, and it is useless without the matching row.
- `Max-Age` is the session idle timeout, default 30 days, capped at the IdP
  refresh token lifetime the backend learns from `refresh_expires_in` (Keycloak)
  or config. Every refresh resets it. An absolute lifetime lives in the session
  row, not the cookie.
- Idle timeout vs Keycloak: without `offline_access`, a Keycloak refresh token
  dies at the realm's SSO Session Idle (30 minutes by default) and SSO Session
  Max. With it, Offline Session Idle and Offline Session Max Limited apply. The
  backend sets `Max-Age` and `absolute_expires_at` to the minimum of its config
  and `refresh_expires_in` from each token response, so it never promises a
  session the realm will refuse. The template realm sets Offline Session Idle
  to 30 days and requests `offline_access`; startup logs a warning when the
  configured idle timeout exceeds the first `refresh_expires_in` seen.
- Local dev: Chrome and Firefox treat `http://localhost` as a secure context and
  accept `Secure` and `__Host-` cookies over plain http there. Safari does not.
  `BrowserSessionConfig.insecure_local_cookie` drops the prefix and `Secure` only
  when every allowed origin is `http://localhost` or `127.0.0.1`; startup fails
  otherwise. Playwright runs Chromium, so CI uses the production attributes.

### 3. CSRF

All three endpoints are `POST` and require all of:

1. `SameSite=Strict`, so a cross-site request carries no cookie.
2. Header `X-Baukit-Session: 1`. A cross-origin form cannot set it, and a
   cross-origin `fetch` with it triggers a preflight that CORS denies.
3. `Origin` present and in `HttpOptions.allowed_origins`, the same list the CORS
   layer uses. Missing `Origin` is refused. Same-site but foreign origins
   (another product on a sibling port or subdomain) fail here, which
   `SameSite` alone does not stop.

`X-Baukit-Session` joins the CORS allowed headers in baukit-http. The router
refuses to build when `allowed_origins` is empty or `allow_credentials` is off,
because then the browser would drop the cookie.

Only the session client sends cookies: its three `fetch` calls use
`credentials: 'include'`. `authenticated-api.ts` and every other API call keep
the default `credentials: 'same-origin'` and send the bearer header, so the
`Path=/` cookie never reaches a non-session route cross-origin. On a single
origin deployment it does ride along, and the API ignores it there.

Rejected: a double-submit CSRF token. It needs a readable cookie or a token in
storage, which is what this ADR removes, and adds nothing over 2 and 3.

### Rate limiting

`/auth/session/exchange` and `/auth/session/refresh` go through
`baukit-ratelimit` with two `authenticated_route_group`-style buckets on the
shared Redis store:

- per client IP (`RateLimitOptions::ip_key`, honouring `trusted_proxy_hops`):
  group `auth-session-ip`, 30 requests per minute, burst 10.
- per session id (the id part of the cookie, never the secret): group
  `auth-session-id`, 12 requests per minute, burst 4. Exchange has no session
  yet and uses only the IP bucket.

A baukit-ratelimit `Quota` admits `requests_per_minute + burst` from a full
bucket, so a client gets 40 IP requests and 16 session requests in its first
minute, then the per-minute rate. baukit-auth defines the limiter as a port
(`BrowserSessionRateLimiter`), because baukit-ratelimit depends on baukit-auth.
The adapter lives in baukit-ratelimit behind its `browser-session` feature.

These are the defaults. `BrowserSessionConfig::with_rate_limits` overrides both
buckets, and the template exposes the four numbers under
`[auth.browser_session]`. Only the local config raises them, for end-to-end
suites that sign in hundreds of times from one loopback address.

Logout is limited per IP only. Fail mode follows `RateLimitOptions.fail_mode`
(fail closed in the template). A refused request gets `429` with
`Retry-After`; the client treats it like a network error and keeps the account
hint.

### 4. Refresh token at rest: server-side session table

The cookie holds an opaque `session_id.secret` (128-bit id, 256-bit secret).
Table `baukit_auth_browser_sessions` in baukit-auth's migrations stores:
`id`, `secret_hash` (SHA-256), `family_id`, `subject`, `refresh_token` and
`id_token` encrypted with `CredentialCipher` (AAD = id + subject), `created_at`,
`last_used_at`, `absolute_expires_at`, `revoked_at`, `rotated_to`,
`grace_access_token` (encrypted, AAD = row id + subject), `grace_until`.

Refresh rotates: it creates a new row in the same family, marks the old one
`rotated_to`, and stores the IdP's rotated refresh token if it sent one. A
refresh locks the presented row (`SELECT ... FOR UPDATE`), so a second request
with the same secret waits until the first commits.

Two tabs share the cookie jar, so the losing tab gets the successor cookie as
soon as the winner's response lands. What it lacks is an access token. The
winner therefore stores the access token it returns in the successor row as
`grace_access_token`, with `grace_until = now + 10 s`. When a request presents
an old secret whose hash matches a row with `rotated_to` set:

- inside the successor's `grace_until`: respond `200` with the successor's
  grace access token and its `expiresAt`, and no `Set-Cookie`. No IdP call, no
  new row.
- after it: reuse. The backend revokes the whole family, revokes the refresh
  token at the IdP, clears the cookie and returns 401.

A later rotation of the successor nulls its `grace_access_token`. The grace
token is an access token already sent to the same browser, so storing it for
10 seconds widens nothing. Rows go through the same
retention purge as other baukit tables; `subject` lets data-subject erasure and
"sign out everywhere" delete all of a user's sessions.

| | Session table (chosen) | Stateless AEAD cookie |
|---|---|---|
| Revoke one session or all for a user | yes, delete rows | no, cookie stays valid until IdP revokes |
| Reuse detection | yes, `rotated_to` | no, an old cookie is indistinguishable from the new one |
| Erasure | rows keyed by subject | nothing to erase, but nothing to list |
| Cost | one PostgreSQL row per browser | none |

Every product backend with auth already runs PostgreSQL, so the cost is small.
Rejected: stateless cookies, because reuse detection is the point of rotating.

### 5. Access token in the browser

- Memory only, inside `BrowserSessionClient` (new class in auth-web, replacing
  `OidcClient`'s token storage). Lifetime is the IdP's, 5 minutes in the
  template realm; refresh starts at `expiresAt - min(30 s, lifetime / 2)` as
  today.
- On reload the client calls `refresh` once. A 401 means signed out.
- Tabs: a refresh runs under `navigator.locks.request('<prefix>:refresh')`.
  The winner posts `{ accessToken, expiresAt, subject }` on a
  `BroadcastChannel('<prefix>:session')`; waiting tabs take that token instead
  of refreshing. Logout and `session_expired` are broadcast too. Without Web
  Locks the grace window in section 4 covers races.
- `sessionStorage` still holds the PKCE transaction for the redirect. Nothing
  token-shaped goes to `localStorage`, `sessionStorage`, IndexedDB or caches.
  `localStorage[<prefix>:account]` keeps only `{ subject, signedInAt }`, so an
  offline reload can open the right local profile and show "offline, signed in
  as ...".
- Offline: local data opens without a token. Sync and API calls queue until
  `refresh` succeeds. A network error on refresh is retryable and does not clear
  the account hint; only a 401 does.
- Service worker: may cache the app shell and public assets. It must not cache
  `/auth/session/*` or any response to an `Authorization` request, must not
  read or hold tokens, and `pwa-web` adds `/auth/session/` to its network-only
  denylist.

### 6. Hosted-session providers are out of scope

Clerk and WorkOS AuthKit keep their own session in their SDK and cookies on the
vendor's domain; their JS never exposes a long-lived refresh token to product
code, and the products hold only short session JWTs fetched per request. A BFF
in front of them would need their backend SDKs and duplicate their session
store. `ClerkWebClient` and `WorkOsWebClient` keep working as today, under the
same CSP. Their own token handling is their vendor's risk, recorded in each
product's threat model (S-10). Generic OIDC (Keycloak, Zitadel, any IdP via
`OidcClient`) is in scope. The suite silent-token broker already holds
memory-only access tokens and has no refresh token, so it stays.

### 7. Security headers and CSP

baukit-http gets `SecurityHeadersLayer` in `layers()`, on by default:
`X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`,
`X-Frame-Options: DENY`, `Content-Security-Policy: default-src 'none';
frame-ancestors 'none'` on API responses, `Cache-Control: no-store` on every
`/auth/session/*` response and on responses to requests with `Authorization`,
and HSTS behind `HttpOptions.hsts` (off for localhost).

The templates ship a CSP for the static server and the Helm chart's web
ingress. The base, shared by every flavor:

```
default-src 'self'; script-src 'self'; img-src 'self' data: blob: <media origin>;
media-src 'self' blob: <media origin>;
connect-src 'self' <api origin> <issuer origin> <posthog origin> <suite hub api origin>;
frame-src <issuer origin>; worker-src 'self'; manifest-src 'self'; object-src 'none';
base-uri 'none'; form-action 'self' <issuer origin>; frame-ancestors 'none';
upgrade-insecure-requests
```

Per flavor, `style-src` differs:

| Flavor | Built by | `style-src` | Why |
|---|---|---|---|
| web (`templates/web/web`) | Vite | `'self'` | CSS is emitted as files |
| mobile-pwa (`templates/mobile/mobile`, `web.output: 'single'` in `app.config.ts`) | Expo Metro web export | `'self' 'unsafe-inline'` | react-native-web injects `<style>` elements at runtime |

Scripts never get `'unsafe-inline'`. If the Expo export or the Vite build
emits an inline `<script>`, the build step computes its SHA-256 and adds it as
a `'sha256-...'` source; the implementation task checks the current exports
and adds a build test that fails on an inline script without a matching hash.
`<media origin>` is the host behind the njs media-grant verifier
(`deploy/media-grants`), present only when the product serves media.
`<suite hub api origin>` is present only for suite products, which call the
hub's API through `@baukit/suite-client`; the silent-token iframe origin is the
issuer and is covered by `frame-src`. Clerk and WorkOS flavors add their
vendor hosts. Vite dev mode gets a relaxed variant (`'unsafe-inline'` styles,
websocket for HMR), never the production build. Trusted Types is a later step.
`@baukit/csp` builds the policy; the templates keep only their origins.

Next to the CSP, `vite preview` and the chart's nginx send
`Cross-Origin-Opener-Policy: same-origin` and
`Cross-Origin-Resource-Policy: same-origin`. The template sign-in is a
same-window redirect and the silent-token frame talks to `window.parent`,
which COOP does not touch. The interactive `SilentTokenBroker` popup reads
`window.opener` on its callback page after a cross-origin hop to the issuer.
No template wires it. Under `same-origin` the popup inherits the opener's COOP,
so the hop to the issuer switches browsing context groups and `opener` is gone
before the callback loads. A product that adds that popup sends
`same-origin-allow-popups` on its app pages and no COOP on the callback page.
That is not enough on WebKit. Safari severs `window.opener` after a
cross-origin navigation under any COOP value, including an absent header on the
callback page, so `window.opener.postMessage` cannot carry the result there.
The callback page posts the authorization result, with the `state` it received,
on a `BroadcastChannel` named per product. The opener listens on the same
channel, accepts only a message whose `state` matches its pending transaction,
and closes the channel after the first match. This works on all engines and
needs no opener relationship.
COEP stays unset: `require-corp` blocks third-party images and analytics
unless every subresource opts in.

### 8. Same-site requirement

Every product serves web and API on one registrable domain, for example
`app.example.com` and `api.example.com`, or one origin with `/api`. Startup of
the BFF router checks that each allowed origin shares the registrable domain of
`BrowserSessionConfig.public_api_origin` (public suffix list) and fails
otherwise. Local dev on `localhost` with two ports passes.

Same-site is decided on the host string, not on the machine. `localhost` and
`127.0.0.1` reach the same server but are different sites, so a web app opened
on `http://127.0.0.1:5173` sending credentialed requests to
`http://localhost:8080` gets no `SameSite=Strict` cookie and fails the CORS
origin check. Open the web app and the API with the same host name in local dev
and list that exact origin in the backend's allowed origins.

### 9. Mobile web build

A generated mobile app with `--pwa` and no `--web` app exports an Expo web
build. That build runs in a browser, so the rules above apply to it as well.
`expo-secure-store` has no secure backend on web, and the old `auth-storage.ts`
fell back to `localStorage` there.

- `auth-storage.ts` throws `WebTokenStorageRefused` on web for every call. It
  has no browser fallback, so a native client loaded by mistake fails loudly.
- OIDC (Keycloak and the local realm) uses `BrowserSessionClient` against the
  backend `/auth/session` routes. The backend template enables those routes and
  the `-web` Keycloak client when either `web` or `pwa` is set.
- Clerk uses `ClerkWebClient` with no token cache. `@clerk/expo` on web ignores
  `tokenCache`, loads clerk-js from a CDN that the CSP blocks, and refuses
  http(s) redirect URLs, so the browser SDK is the only option that works.
- WorkOS uses `WorkOsWebClient` (authkit-js). The native WorkOS flow calls the
  vendor's JSON `authenticate` endpoint rather than OIDC, so the BFF cannot
  serve it. Section 6 applies, as it does for the web template.
- iOS and Android keep `expo-secure-store` unchanged.

A guard test in the template (`src/web-sign-in-storage.test.ts`) signs in on
web against a fake browser and fails if any `localStorage` or `sessionStorage`
write carries a token, or if IndexedDB, Cache Storage or SecureStore is used.

## Migration

No product is live, so baukit 0.19.0 removes token storage from `OidcClient`
without a compatibility mode. Each product with an OIDC web client must:

1. Mount `browser_session::router` under `/auth/session`, add
   `BrowserSessionConfig` (issuer, client id, redirect URIs, cookie name,
   idle and absolute timeouts, credential-vault key) and run the new migration.
   Enable `baukit-ratelimit` with Redis if the product has not yet.
2. Set `allow_credentials` and the web origin in `allowed_origins`.
3. Replace `OidcClient` with `BrowserSessionClient` and pass the API base URL.
   `getAccessToken`, `handleCallback`, `onSessionExpired` and `clearSession`
   keep their names.
4. Move the refresh lifetime from the web client to the backend: Keycloak web
   client gets `offline_access` only if the product wants 30-day sessions.
5. Serve the template CSP and check that the deploy keeps web and API same-site.
6. Delete any product code that reads `<prefix>:tokens`.

Clerk and WorkOS products only adopt the CSP and baukit-http headers.
Generated products pick all of this up from the template.

## Test plan

- Unit (Rust): cookie attributes per mode; `insecure_local_cookie` refused with
  a non-local origin; CSRF matrix (missing header, missing Origin, foreign
  Origin, same-site foreign port) each 403; rotation, grace window, reuse revokes
  the family; AAD mismatch fails decrypt; same-site startup check.
- Unit (TypeScript): `BrowserSessionClient` with fake fetch, locks and
  channel; one refresh across three tabs; 401 vs network error; a test that
  spies on `Storage.setItem`, IndexedDB and Cache API writes and fails on any
  value containing the access token, `refresh_token` or `eyJ`.
- Integration (testcontainers Keycloak + PostgreSQL, `#[ignore]`d, run with
  `--include-ignored`): full code exchange, refresh with Keycloak rotation,
  logout revokes at Keycloak (old refresh token then fails at the IdP), reuse of
  a rotated cookie revokes the session, access token rejected by the verifier
  after its `exp`.
- Browser e2e (template Playwright stack, `qa-auth-*.spec.ts`): login, reload
  keeps the session, two tabs refresh once, offline reload opens local data,
  logout in one tab signs out the other, `localStorage` and IndexedDB hold no
  token after login.
- Negative e2e: a page on a foreign origin posts a form and a `fetch` to
  `/auth/session/refresh` and gets no token; a stolen access token stops working
  after expiry; replaying a captured old cookie ends the session.
- Grace window (Rust unit, fake clock): old secret inside 10 s returns the
  successor's grace token with no `Set-Cookie` and no IdP call; at 10 s plus
  one tick it revokes the family; two concurrent refreshes produce exactly one
  IdP call and one successor; rotating the successor clears its grace token;
  AAD from another row fails to decrypt the grace token.
- Rate limit (Rust integration with Redis testcontainer): the 41st exchange per
  minute from one IP and the 17th refresh per session id return 429 with
  `Retry-After`; a second session id from the same IP still passes the session
  bucket.
- Credentials (TS unit): only session calls use `credentials: 'include'`; an
  API call made through `authenticated-api.ts` uses `same-origin`.
- CSP in Playwright, every flavor (web, mobile-pwa, combined, auth, clerk,
  workos): extend `templates/web/web/e2e/tests/console-warnings.ts`, which today
  collects only `warning` messages, to also collect `error` messages and
  `securitypolicyviolation` events, and fail the QA run on any CSP violation.
  Allowlist entries are not allowed for CSP.
- Fixture gate: `curl -I` on the built web shows the CSP; API responses carry
  `nosniff` and `no-store` on `/auth/session/*`.

## Implementation tasks

Four agents, disjoint files:

1. **baukit-auth BFF** (`rust/crates/baukit-auth/src/browser_session/`,
   new migration, `Cargo.toml`, crate tests): config, session store on
   PostgreSQL with credential-vault encryption, IdP token client, router, CSRF
   guard, same-site check, grace window, rate-limit wiring, Keycloak and Redis
   integration tests.
2. **baukit-http headers** (`rust/crates/baukit-http/src/middleware.rs`,
   `options.rs`, new `security_headers.rs`, tests): `SecurityHeadersLayer`,
   HSTS option, `X-Baukit-Session` in CORS allowed headers.
3. **auth-web and pwa-web** (`typescript/packages/auth-web`,
   `typescript/packages/pwa-web`): `BrowserSessionClient`, remove token storage
   from `OidcClient`, locks and BroadcastChannel coordination, storage-write
   test, service-worker denylist.
4. **Templates and chart** (`templates/`, `cli/tests/snapshots`,
   `deploy/helm`): mount the router, config and env, swap the web client, CSP
   per flavor for the static server and ingress, inline-script hash check, CSP
   violation collection in `console-warnings.ts`, e2e specs, fixture header
   check. Starts
   after 1 to 3 publish their public signatures, which this ADR fixes.

## Consequences

- A stolen access token is good for at most its lifetime (5 minutes in the
  template realm). XSS can still act as the user while the page is open, which
  CSP narrows but does not remove.
- Each OIDC product backend now handles IdP token traffic and holds encrypted
  refresh tokens, so its credential-vault key becomes a high-value secret.
- Web and API can no longer live on unrelated domains.
- Reload costs one refresh round trip before the first API call.
- Clerk and WorkOS products remain on their vendor's browser session model.
