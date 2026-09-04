# Evidence for browser identity composition

## Source product files

The product revisions examined were Tiefgang `861cf0a994d5e63ec245e645023c80575759c191`, Leitbild `25eda071f0e2538b78a3ea62129a73770d506e2b`, Redemut `b4e8a9872595260d3f26af7d8d085aac98485e51`, and Eigenruhe `36b468d015f4aebd83a11bd662c7ff82124711fb`.

- Redemut: `web/src/auth.ts`, `web/src/auth.test.ts`, `web/src/auth-popup-callback.tsx`, `web/src/auth-popup-protocol.ts`, `web/src/startup.tsx`, `mobile/src/auth.ts`, and `mobile/test/auth.test.ts`.
- Leitbild: `web/src/api-tokens.tsx`, `web/src/api-tokens.test.tsx`, `backend/crates/leitbild-api/src/api_token.rs`, `backend/crates/leitbild-postgres/src/api_token.rs`, and `backend/migrations/0011_create_api_tokens.sql`.
- Tiefgang: `mobile/src/auth/storage.ts` and `mobile/src/auth/oidc.ts`.
- Eigenruhe: `mobile/src/auth-storage.ts`, `mobile/src/auth-storage.test.ts`, and `mobile/src/auth.ts`.
- Baukit comparison: [`auth-web`](../../typescript/packages/auth-web/src/index.ts), [`auth-native`](../../typescript/packages/auth-native/src/index.ts), [`auth-node`](../../typescript/packages/auth-node/src/index.ts), and [`baukit-auth`](../../rust/crates/baukit-auth/src/lib.rs).

## Observed failure or repeated glue

Redemut's cached account check accepts an undecodable token as a match. Its popup completion signal is global to the origin, has no attempt ID or timeout, and permits overlapping calls. Tiefgang and Eigenruhe repeat the same web versus native storage adapter. Product auth wrappers separately own proactive refresh and identity cleanup. Leitbild supplies a sound personal-token management example, but its server-only OIDC guard is an essential security rule rather than UI behavior.

## Baukit owner

`@baukit/auth-web` owns browser OIDC mechanics and is the possible owner of a later optional popup coordinator. `@baukit/auth-native` owns the secure storage port and native session mechanics. `baukit-auth` owns personal-token formatting, digest verification, store operations, principal establishment, and credential-kind metadata. Platform documentation should own the account-bootstrap, token-management, and universal-storage recipes.

## Public types and errors

No public code is added by this study. A later account contract needs signed-out, confirming, confirmed, unavailable, absent, and blocked-mismatch states, with typed unavailable, absent, undecodable, and mismatch outcomes. A later popup coordinator needs completed, full-page fallback, closed, timed-out, callback-failed, and already-active outcomes. Existing `IssuedApiToken`, `ApiTokenStore`, `ApiTokenVerifier`, and `Principal::api_token` are sufficient for the token recipe.

## Product-owned inputs

Products supply the account API and DTO, provisioning route, retry copy, repository factory, callback route, popup presentation, provider configuration, token names and limits, permission mapping, storage prefix, browser storage choice, proactive refresh timing, analytics identity, and all UI.

## Concurrency, failure, privacy, and cleanup cases

The contract must cover one active popup, correlated events, wrong origin, wrong source, stale events, timeout, closure, callback failure, blocked-popup fallback, and complete listener and timer cleanup. Account bootstrap must cover retryable backend failure, explicit absence, invalid OIDC session, undecodable claims, backend mismatch, cache mismatch, logout, and identity change. Refresh remains single-flight and fenced against late completion. Token secrets are returned once and excluded from logs, analytics, and storage. Logout clears auth state, account bindings, timers, analytics identity, and identity-scoped local contexts before navigation.

## Supported runtimes

The account and popup contract targets modern browsers. The storage recipe covers browser, Expo web, iOS, and Android. Node uses `@baukit/auth-node` and remains outside the universal client storage recipe. Rust owns server-side token verification and management composition.

## Product adoption change

This study deletes no product code. After a tested optional coordinator ships, Redemut can remove `PopupOidcClient`, `waitForPopupCompletion`, and the local protocol constants from `web/src/auth.ts` and `web/src/auth-popup-protocol.ts`; its callback page and copy stay local. A later account coordinator could replace the bootstrap state in `web/src/auth.ts`, while the account API and learning-context mount remain local. The storage and token decisions are recipes, so Tiefgang's `mobile/src/auth/storage.ts`, Eigenruhe's `mobile/src/auth-storage.ts`, and Leitbild's token handlers remain product composition.

## Throwaway experiments

None. The study used source inspection only.
