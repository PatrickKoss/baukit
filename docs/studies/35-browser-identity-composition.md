# Browser identity composition study

## Question and scope

This study asks which parts of Redemut's browser account bootstrap and popup OIDC flow, Leitbild's personal token management, and the universal auth storage used by Tiefgang, Eigenruhe, and Redemut belong in Baukit. It compares those products with [`@baukit/auth-web`](../../typescript/packages/auth-web/src/index.ts), [`@baukit/auth-native`](../../typescript/packages/auth-native/src/index.ts), [`@baukit/auth-node`](../../typescript/packages/auth-node/src/index.ts), and [`baukit-auth`](../../rust/crates/baukit-auth/src/lib.rs). The study defines state machines and recipes only. It does not propose product account schemas, routes, token scopes, UI, or provider policy.

## Evidence table

| Product or owner | File | What it does | What varies |
| --- | --- | --- | --- |
| Redemut | `/home/patrick/projects/redemut/web/src/auth.ts`, `BaukitAuthProvider` | Finishes the OIDC callback, loads the backend account, caches the confirmed account identity, retries 429 and server failures, and exposes degraded availability. | Account DTOs, retry copy, and the local `AuthUser` shape are Redemut-specific. The cache check currently accepts an undecodable token, which is unsafe. |
| Redemut | `/home/patrick/projects/redemut/web/src/startup.tsx`, `openAuthenticatedWebLearningContext` | Obtains a token and opens identity-scoped local learning repositories with the backend account issuer and subject. | Repository types and content preparation are product code. The repository mount depends on the account bootstrap being fail-closed. |
| Redemut | `/home/patrick/projects/redemut/web/src/auth.ts`, `PopupOidcClient` and `waitForPopupCompletion` | Opens a popup during a user gesture, moves the OIDC transaction into popup session storage, checks message origin and source, detects window closure, cleans listeners, and falls back to full-page navigation when the popup cannot be used. | Window size, callback route, and UI are local. The completion signal has no attempt correlation or timeout, and the client does not reject overlapping attempts. |
| Redemut | `/home/patrick/projects/redemut/web/src/auth-popup-callback.tsx` and `auth-popup-protocol.ts` | Completes login, writes a global local-storage signal, posts a same-origin completion message, closes the popup, and shows local progress or failure copy. | Copy and fallback route are product choices. The signal contains no coordinator-issued attempt ID. |
| Leitbild | `/home/patrick/projects/leitbild/web/src/api-tokens.tsx` and `api-tokens.test.tsx` | Lists tokens, creates one, displays and copies the plaintext once, confirms revocation, and keeps the plaintext visible after a clipboard failure. | Names, expiry choices, active-token limit copy, and settings UI are product policy. |
| Leitbild | `/home/patrick/projects/leitbild/backend/crates/leitbild-api/src/api_token.rs` | Requires an interactive OIDC principal for list, create, and revoke, resolves the owner server-side, and returns the created secret only in the create response. | Routes, response models, owner lookup, and safe error codes are product-owned. |
| Leitbild | `/home/patrick/projects/leitbild/backend/crates/leitbild-postgres/src/api_token.rs` and `backend/migrations/0011_create_api_tokens.sql` | Stores only a digest, lists and revokes by owner, and enforces the product's active-token limit. | The table, limit, token naming rules, and database errors belong to Leitbild. There is no generic scopes column. |
| Tiefgang | `/home/patrick/projects/tiefgang/mobile/src/auth/storage.ts` and `mobile/src/auth/oidc.ts` | Selects browser local storage on web and Expo SecureStore on native, including safe encoding for native storage keys, then adapts it to `SecureStoragePort`. | Storage prefix and Expo composition are product-local. |
| Eigenruhe | `/home/patrick/projects/eigenruhe/mobile/src/auth-storage.ts`, `auth-storage.test.ts`, and `auth.ts` | Implements the same runtime storage selection, tests both branches, and owns a proactive refresh timer. | Storage keys, timer wiring, and session-to-product identity mapping are local. |
| Redemut | `/home/patrick/projects/redemut/mobile/src/auth.ts` and `mobile/test/auth.test.ts` | Uses the Expo adapter, schedules proactive refresh, shares concurrent refresh, preserves the session after transient failures, clears terminal failures, and clears locally before provider logout. | Refresh timing, app state integration, and UI state are local. |
| Baukit | [`typescript/packages/auth-web/src/index.ts`](../../typescript/packages/auth-web/src/index.ts) and [`README.md`](../../typescript/packages/auth-web/README.md) | Owns browser PKCE state, callback validation, browser token and transaction storage, concurrent refresh sharing, a session revision fence, terminal expiry, and local-first logout. | The package does not own backend account confirmation or popup coordination. Browser storage remains exposed to the page's script security boundary. |
| Baukit | [`typescript/packages/auth-native/src/index.ts`](../../typescript/packages/auth-native/src/index.ts), [`expo.ts`](../../typescript/packages/auth-native/src/expo.ts), and [`README.md`](../../typescript/packages/auth-native/README.md) | Defines `SecureStoragePort`, provides Expo SecureStore composition, deduplicates refresh, clears corrupt or terminal sessions, and clears locally before provider logout. | Universal Expo apps must still choose the web storage branch and own proactive refresh scheduling. |
| Baukit | [`rust/crates/baukit-auth/src/api_token.rs`](../../rust/crates/baukit-auth/src/api_token.rs) and [`axum_integration.rs`](../../rust/crates/baukit-auth/src/axum_integration.rs) | Defines one-time `IssuedApiToken` secrets, digest verification, owner-scoped `ApiTokenStore` list and revoke operations, `ApiTokenVerifier`, and `establish_principal`. | Products own persistence, issuance limits, authorization scopes, routes, and the mapping from a verified token to domain permissions. |
| Baukit | [`typescript/packages/auth-node/README.md`](../../typescript/packages/auth-node/README.md) and [`src/cache.ts`](../../typescript/packages/auth-node/src/cache.ts) | Provides a Node device-flow client with an atomic, locked profile cache and restrictive file permissions. | This is a Node CLI storage model. It is not a browser or native storage abstraction and should not be included in a universal client recipe. |

## Candidate interface or contract sketch

### Server-confirmed account bootstrap

The account coordinator should expose one state at a time:

| State | Entry condition | Allowed behavior | Exit |
| --- | --- | --- | --- |
| `signed-out` | No OIDC session is present, or OIDC verification ended terminally. | Clear the cached account binding and do not mount identity-scoped repositories. | A new OIDC session starts `confirming-account`. |
| `confirming-account` | An OIDC session exists and yields a non-empty issuer and subject. | Ask the backend for the account. Do not mount local repositories yet. | A matching account becomes `confirmed`; a typed absence becomes `account-absent`; a retryable failure may become `backend-unavailable`; any undecodable or conflicting identity becomes `blocked-identity-mismatch`. |
| `confirmed` | The backend account issuer and subject exactly match the OIDC session issuer and subject. | Persist an account cache that contains the issuer, subject, and product account data. Mount repositories with the confirmed issuer and subject, never a display name or account label. | Logout clears the cache. A changed subject restarts confirmation. |
| `backend-unavailable` | The backend has a transport, 429, or server failure, and a prior backend-confirmed cache is bound to the same successfully decoded issuer and subject. | Reuse that cache, report degraded availability, and retry with bounded backoff. | A matching response returns to `confirmed`; absence or mismatch leaves the mounted context and blocks it; terminal OIDC failure becomes `signed-out`. |
| `account-absent` | The backend explicitly reports that no product account exists for the OIDC principal. | Do not reuse another cache and do not mount repositories. Offer only a product-owned provisioning or support route. | Successful provisioning restarts confirmation. Logout becomes `signed-out`. |
| `blocked-identity-mismatch` | The OIDC identity is undecodable, the backend account differs, the cache differs, or identity changes while confirmation is pending. | Fail closed, clear account-derived state, cancel retries, close the old local context, and require a fresh confirmation. | Only a newly confirmed matching account becomes `confirmed`. |

The cached binding is an availability aid, not authorization. The backend still verifies the bearer token on every protected request. A browser-decoded token is acceptable for cache comparison only after the auth client established the OIDC session and the token contains well-formed issuer and subject claims. Opaque or malformed access tokens disable cached reuse.

Security correction: Redemut's `identityMatchesToken` treats a failed decode as a match. The shared contract must return `false`, clear the cache, and block repository mounting when claims cannot be decoded. It must also compare the backend account with the session identity before accepting or caching it.

### Popup OIDC login

The coordinator recipe is:

1. Reject a second login while an attempt is active.
2. Create a cryptographically random attempt ID before opening the popup. This is separate from the OIDC `state` that protects the authorization response.
3. Open the blank popup inside the user gesture. If opening fails, cancel popup bookkeeping and use the normal full-page OIDC redirect.
4. Store the attempt ID with the popup transaction and require the callback to return that exact ID in both `postMessage` and the storage-event fallback.
5. Accept `postMessage` only from the exact configured application origin, the exact popup window, and a message with the expected type and attempt ID. Accept a storage event only from the expected storage area, key, and attempt ID.
6. Race successful completion against a bounded timeout and popup-close polling. Callback failure must produce an explicit failed result rather than leaving the opener waiting.
7. On every outcome, remove message and storage listeners, stop timers, remove the attempt-scoped completion value, close the popup if possible, and release the single active attempt.
8. Return typed results for completed, blocked-popup fallback, closed, timed out, callback failed, and already active. The product selects copy, routes, provider settings, and popup dimensions.

Security correction: Redemut's origin and source checks are sound, but its global completion value can settle the wrong tab or a later attempt. A Baukit coordinator must bind every completion path to a fresh attempt ID, enforce one active attempt, and time out.

### Personal token management

The server recipe is:

1. Run `establish_principal` before every management route.
2. Require an OIDC principal for list, create, and revoke. Reject a principal with `api_token()` present, even if that token can call other product routes.
3. Resolve the owner from the verified OIDC issuer and subject. Do not accept an owner ID from the request.
4. Call `ApiTokenService` and an owner-scoped `ApiTokenStore`. Return metadata for list and revoke.
5. Return `IssuedApiToken.secret` only from a successful create response. Do not persist, log, re-fetch, or place the secret in analytics. A failed clipboard operation may keep the already displayed value in the current page state.
6. If product permissions are needed, wrap verification and map the verified token ID to product authorization data. Keep that mapping and its tables in the product.
7. Do not add a generic scopes input or column until a second product has the same scope data and enforcement rules.

Security correction: the interactive-principal check belongs on the server after authentication. Hiding controls in the browser is insufficient. A personal token must never list, create, or revoke personal tokens, especially another token that would extend its access.

### Universal auth storage

Use one composition function in a universal app. On native iOS and Android it passes Expo SecureStore to `@baukit/auth-native`. On web it passes a `SecureStoragePort` adapter backed by the browser storage selected by the product. `@baukit/auth-web` may be used directly for a browser-only app. `@baukit/auth-node` keeps its separate file-cache contract.

The session coordinator, not each screen, owns four lifecycle rules:

- Logout and terminal expiry clear every auth-owned token, transaction, account binding, proactive timer, analytics identity, and identity-scoped local context before provider navigation.
- An issuer or subject change closes the old context before opening a new one. Display claims never choose a storage partition.
- One layer owns proactive refresh timing. Calls that merely need a token do not create another timer.
- Concurrent refresh calls share one operation. A generation or revision fence prevents a late refresh from restoring a session after logout or an identity change.

Security correction: the recipe must not describe browser Web Storage as secure storage. It shares the page's script exposure. The correction is explicit runtime selection plus complete cleanup, identity fencing, one proactive-refresh owner, and single-flight refresh. A new universal storage package would hide the material difference between native secure storage and browser storage without removing product composition.

## Required-case coverage

| Required case | Current coverage | Gap or correction |
| --- | --- | --- |
| OIDC session absent or terminal | Both Baukit clients clear terminal sessions; Redemut clears its account cache on expiry. | The account contract must make local-context teardown part of the same transition. |
| Backend-confirmed account | Redemut fetches the account before normal startup. | Compare the returned issuer and subject with the session before caching or mounting. |
| Backend unavailable | Redemut tests 429 and 503 retry, cached degraded startup, recovery, and no-cache failure in `web/src/auth.test.ts`. | Cached reuse must require a successfully decoded exact identity match. |
| Account absent | No source product has a distinct account-absent state. | Add a typed absence transition and keep provisioning local. |
| Identity mismatch or undecodable identity | Redemut validates the cache shape but currently accepts undecodable tokens and has no mismatch test. | Both cases must enter the blocked state and clear account-derived state. |
| Repository partition selection | Redemut startup uses backend account issuer and subject. | It is safe only after the corrected confirmation state; display claims and stale account keys are forbidden. |
| Fresh popup correlation | OIDC `state` is fresh in `@baukit/auth-web`; Redemut's separate popup completion signal is global. | Add a distinct fresh attempt ID to both completion transports. |
| Exact origin and popup source | Redemut checks both for `postMessage`. | Preserve the checks and add exact attempt matching. |
| One active popup attempt | Redemut keeps one popup reference but does not reject overlap. | Return an already-active result before opening another popup. |
| Popup timeout and closure | Redemut polls for closure but has no timeout. | Bound both the waiter and cleanup lifetime. |
| Popup cleanup and full-page fallback | Redemut removes listeners and intervals and falls back when popup navigation is unavailable. | Cleanup must also remove attempt-scoped storage on every result. |
| Popup callback failure and stale storage event | The callback shows failure locally; no opener failure signal or stale-event test exists. | Send a correlated failed result and ignore every stale event. |
| Plaintext token returned once | Leitbild displays only the create response secret and tests clipboard failure behavior. Baukit models the secret as `IssuedApiToken`. | Document log, analytics, and persistence exclusions. |
| OIDC required for token management | Leitbild's `require_oidc_session` covers list, create, and revoke. | Keep the check server-side after `establish_principal`. |
| Owner-scoped list and revoke | Leitbild resolves the owner server-side and its PostgreSQL queries include the owner. Baukit's store port also requires an owner. | Do not add request-supplied owner selection. |
| Personal token cannot mint another | Leitbild rejects principals with an API token. | Preserve this as a compatibility test in any future recipe fixture. |
| Product authorization mapping | `ApiTokenVerifier` retains token metadata on `Principal`. | A product wrapper may map the verified ID to permissions; Baukit must not invent scopes. |
| Native and browser storage selection | Tiefgang and Eigenruhe implement and test runtime selection; Redemut uses separate web and native clients. | Publish composition, not a storage package that implies equal guarantees. |
| Logout cleanup and identity change | Both auth packages clear locally before provider logout and fence pending refresh; product sessions clear identity-scoped state. | The recipe must name all account caches, contexts, timers, and analytics state as cleanup participants. |
| Proactive and concurrent refresh | Eigenruhe and Redemut own proactive timers. Baukit clients share concurrent refresh. | Declare exactly one proactive owner and retain Baukit's single-flight and revision fence. |

## Decision

Decision: contract or recipe. Add a platform contract for server-confirmed account bootstrap and popup coordination, plus personal-token and runtime-storage recipes. Do not implement an `@baukit/auth-web` popup coordinator until the correlation, mismatch, timeout, callback-failure, and cleanup cases have executable tests. Do not add a universal storage package or generic token scopes. The smallest next step is one documentation change with conformance vectors for the account and popup state transitions, followed by a separately reviewed optional coordinator if two products adopt those vectors.

## What stays product-owned

Products keep account records and provisioning, backend account routes, local repository schemas, UI state and copy, callback routes, popup dimensions, identity-provider settings, storage prefixes, proactive refresh timing, analytics identity wiring, personal-token names and limits, authorization scopes, permission tables, database migrations, route error codes, and token-management UI. Redemut's learning context, Leitbild's token settings screen, and each product's account model remain local.
