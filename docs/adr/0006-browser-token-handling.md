# ADR 0006: The browser never holds a refresh token

## Status
Amended 2026-11-27.

## Context

`OidcClient` in `@baukit/auth-web` used to run authorization code with PKCE in the browser and write
access, refresh and ID tokens to `localStorage`. Any XSS bug or compromised npm dependency could read
the refresh token and keep the account for the token's lifetime. No crate, template or chart set a CSP.

Products are local-first PWAs whose local data must open offline without a token. Web and API run on
different origins of one site. Native apps keep tokens in secure storage, and MCP clients send bearer
tokens, so neither touches browser storage.

## Decision

The product backend is a narrow backend-for-frontend for token issuance only. API calls keep
`Authorization: Bearer`, so the verifier, MCP and native apps do not change.

- `baukit-auth` behind its `browser-session` feature has the `browser_session` module. Its router
  mounts under `BROWSER_SESSION_PATH_PREFIX`, `/auth/session`, with `POST exchange`, `refresh` and
  `logout`. The browser still receives the code on its own origin and posts the code and PKCE
  verifier to `exchange`. Session responses carry `accessToken`, `expiresAt` (epoch milliseconds),
  `expiresIn` (seconds) and `subject`. The client computes expiry from `expiresIn` when present, so a
  skewed server clock does not matter.
- The cookie is `__Host-<name>=<id>.<secret>; Path=/; Secure; HttpOnly; SameSite=Strict`, with
  `Max-Age` set to the idle timeout. Defaults are 30 days idle and 90 days absolute, capped by the
  IdP's refresh lifetime. `with_insecure_local_cookie` drops the prefix and `Secure` only when every
  allowed origin is local.
- Every endpoint requires the `X-Baukit-Session: 1` header and an `Origin` in the allowed list, and
  the cookie is `SameSite=Strict`. A failed check returns 403 `csrf_rejected`. The router refuses to
  build without allowed origins or with credentials off, and it fails when an allowed origin is not
  on the registrable domain of `public_api_origin`.
- Table `baukit_auth_browser_sessions` holds one row per rotation, grouped by `family_id`. A row
  stores a SHA-256 of the secret. Its refresh token, ID token and grace access token are sealed
  together as one credential-vault v2 record (`seal_record`), keyed by `record_id`, with the row id
  and subject as AAD. A column cannot be swapped in from another row. Rows written before migration
  `0003_baukit_auth_browser_session_record_ids.sql` have no `record_id` and open field by field.
- Refresh locks the row, rotates it and runs in a detached task, so a dropped request cannot lose a
  token the IdP already rotated. An old secret inside the successor's 10-second `GRACE_WINDOW` gets
  the successor's grace access token and no new cookie. If the successor was never presented, the
  old secret can rotate it once more for up to 5 minutes (`LOST_RESPONSE_CAP`). This recovers a
  response lost after commit. Any other reuse revokes the family and the IdP refresh token.
- `exchange` and `refresh` pass two buckets behind the `BrowserSessionRateLimiter` port. They are
  `auth-session-ip` with 30 per minute and burst 10, and `auth-session-id` with 12 per minute and
  burst 4. Logout uses the IP bucket only. `StoreBrowserSessionRateLimiter` in `baukit-ratelimit`
  resolves the client IP through `resolve_client_ip` with `trusted_proxy_hops`, which defaults to 0,
  and buckets IPv6 by prefix. With no socket peer it returns 429 instead of sharing one unknown
  bucket. `BrowserSessionConfig::with_rate_limits` changes the numbers but not the group names. When
  the store is down in closed mode the request gets 503 with `Retry-After: 1`, because no bucket was
  checked.
- `BrowserSessionClient` in `@baukit/auth-web` keeps the access token in memory and refreshes under
  `navigator.locks`, sharing the result over `BroadcastChannel`. Only its session calls use
  `credentials: 'include'`. `localStorage` holds the account hint `{ subject, signedInAt }` and a
  session generation counter, and `OidcClient` keeps a force-login flag there. None of them is a
  token. `sessionStorage` holds the PKCE transaction. `@baukit/pwa-web` never caches
  `/auth/session/`.
- `SecurityHeadersLayer` in `baukit-http` sets `nosniff`, `no-referrer`, `X-Frame-Options: DENY`, a
  `default-src 'none'` CSP on API responses, and `no-store` on responses to requests with
  `Authorization`. `@baukit/csp` builds the web CSP the templates serve. Scripts never get
  `'unsafe-inline'`.
- On web, the mobile template's `auth-storage.ts` throws `WebTokenStorageRefused`, and OIDC uses the
  same backend routes. Clerk and WorkOS keep their vendor's browser session model.
- Keycloak is the development and test IdP. Production signs in with Clerk or WorkOS. See [ADR 0017](0017-identity-providers-and-bearer-verification.md).

## Enforced by

- `rust/crates/baukit-auth/src/browser_session/tests.rs`: `csrf_matrix_rejects_every_incomplete_request`,
  `production_cookie_has_host_prefix_and_secure_attributes`, `same_site_check_uses_the_public_suffix_list`,
  `old_secret_after_the_grace_window_revokes_the_family`, `concurrent_refreshes_make_one_idp_call_and_one_successor`,
  `rate_limits_use_the_ip_and_session_buckets`.
- `rust/crates/baukit-auth/src/browser_session/refresh_tests.rs`: `dropped_refresh_request_still_stores_the_rotated_token`,
  `lost_refresh_response_is_recovered_by_the_previous_cookie`, `refresh_response_carries_relative_expiry`.
- `rust/crates/baukit-auth/src/browser_session/record_tests.rs`:
  `v2_rows_reject_tampering_and_v1_rows_written_before_the_migration_still_open`.
- `rust/crates/baukit-ratelimit/src/browser_session.rs`: `missing_peer_is_limited_instead_of_sharing_an_unknown_bucket`.
- `typescript/packages/auth-web/src/browser-session-storage.test.ts`: "never writes a token to
  localStorage, sessionStorage, IndexedDB or the Cache API".
- `typescript/packages/auth-web/src/browser-session-expiry.test.ts`: "trusts expiresIn over expiresAt
  when the server clock is skewed".
- The ast-grep rule `lint/ast-grep/rules/ts-token-web-storage.yml`, run by `baukit check patterns`.
- `models/BffSessionFamily/` models the session family in TLA+. Its README still lists
  `residual/LostResponse.cfg` as an open violation. The code now handles that case, as the
  `lost_refresh_response_is_recovered_by_the_previous_cookie` test shows. The model has not been
  updated.

## Consequences

A stolen access token works for its lifetime, 5 minutes in the template realm. XSS can still act as
the user while the page is open, and the CSP narrows that. Each OIDC backend handles IdP token
traffic and holds sealed refresh tokens, so its credential-vault key is a high-value secret. Web and
API must share a registrable domain. A reload costs one refresh round trip.

## History

- 2026-10-13, commit `20e692df`: `OidcClient` capped its refresh window by the token
  lifetime.
- 2026-11-07, commit `1c56bc0b`: accepted. It replaced browser token storage in `OidcClient`
  and set the endpoints, cookie, CSRF rules, session table and CSP.
- 2026-11-25, commit `fc8e89c7`: refresh became cancel-safe, `expiresIn` joined the response, and an
  unused head can be rotated once to recover a lost response.
- 2026-11-25, commits `3ee7df1f`, `86375d2f` and `426c56a9`: `trusted_proxy_hops` default went
  from 1 to 0, generated servers gained `ConnectInfo`, and the limiter fails closed without a peer.
  Before that, every client of a generated API shared one IP bucket.
- 2026-11-26, commit `60aeee69`: session tokens sealed as one vault v2 record instead of one
  ciphertext per column.
- 2026-11-27: amended to the code. The migration steps and implementation task list were removed.
