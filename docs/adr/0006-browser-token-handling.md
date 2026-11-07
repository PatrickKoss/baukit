# ADR 0006: Browser token handling

## Status

Proposed, 2026-11-07. Implements decision D2 of the
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
entry in `BrowserSessionConfig.redirect_uris`. Errors use the baukit-http
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

Rejected: a double-submit CSRF token. It needs a readable cookie or a token in
storage, which is what this ADR removes, and adds nothing over 2 and 3.

### 4. Refresh token at rest: server-side session table

The cookie holds an opaque `session_id.secret` (128-bit id, 256-bit secret).
Table `baukit_auth_browser_sessions` in baukit-auth's migrations stores:
`id`, `secret_hash` (SHA-256), `family_id`, `subject`, `refresh_token` and
`id_token` encrypted with `CredentialCipher` (AAD = id + subject), `created_at`,
`last_used_at`, `absolute_expires_at`, `revoked_at`, `rotated_to`.

Refresh rotates: it creates a new row in the same family, marks the old one
`rotated_to`, and stores the IdP's rotated refresh token if it sent one. A
request that presents an already rotated secret is reuse: the backend revokes
the whole family, revokes the refresh token at the IdP, and returns 401. A
10-second grace window returns the successor for the same old secret, so two
tabs racing a refresh do not log the user out. Rows go through the same
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

The web template ships a CSP for its static server and the Helm chart's web
ingress:

```
default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:;
connect-src 'self' <api origin> <issuer origin> <posthog origin>; frame-src <issuer origin>;
worker-src 'self'; manifest-src 'self'; object-src 'none'; base-uri 'none';
form-action 'self' <issuer origin>; frame-ancestors 'none'; upgrade-insecure-requests
```

`frame-src` covers the silent-token iframe. Clerk and WorkOS flavors add their
vendor hosts. Vite dev mode gets a relaxed variant (`'unsafe-inline'` styles,
websocket for HMR), never the production build. Trusted Types is a later step.

### 8. Same-site requirement

Every product serves web and API on one registrable domain, for example
`app.example.com` and `api.example.com`, or one origin with `/api`. Startup of
the BFF router checks that each allowed origin shares the registrable domain of
`BrowserSessionConfig.public_api_origin` (public suffix list) and fails
otherwise. Local dev on `localhost` with two ports passes.

## Migration

No product is live, so baukit 0.19.0 removes token storage from `OidcClient`
without a compatibility mode. Each product with an OIDC web client must:

1. Mount `browser_session::router` under `/auth/session`, add
   `BrowserSessionConfig` (issuer, client id, redirect URIs, cookie name,
   idle and absolute timeouts, credential-vault key) and run the new migration.
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
- Fixture gate: `curl -I` on the built web shows the CSP; API responses carry
  `nosniff` and `no-store` on `/auth/session/*`.

## Implementation tasks

Four agents, disjoint files:

1. **baukit-auth BFF** (`rust/crates/baukit-auth/src/browser_session/`,
   new migration, `Cargo.toml`, crate tests): config, session store on
   PostgreSQL with credential-vault encryption, IdP token client, router, CSRF
   guard, same-site check, Keycloak integration tests.
2. **baukit-http headers** (`rust/crates/baukit-http/src/middleware.rs`,
   `options.rs`, new `security_headers.rs`, tests): `SecurityHeadersLayer`,
   HSTS option, `X-Baukit-Session` in CORS allowed headers.
3. **auth-web and pwa-web** (`typescript/packages/auth-web`,
   `typescript/packages/pwa-web`): `BrowserSessionClient`, remove token storage
   from `OidcClient`, locks and BroadcastChannel coordination, storage-write
   test, service-worker denylist.
4. **Templates and chart** (`templates/`, `cli/tests/snapshots`,
   `deploy/helm`): mount the router, config and env, swap the web client, CSP
   for the static server and ingress, e2e specs, fixture header check. Starts
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
