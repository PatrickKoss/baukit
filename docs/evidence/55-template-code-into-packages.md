# Template code into packages evidence

Plan item 19, "Move copied template code into packages". Six helpers, one commit each.

This records earlier package work. The TypeScript MCP template has since
been removed. Current MCP adoption follows the [migration guide](../migrations/mcp-stdio-to-remote.md).

## Source revisions

- Baukit baseline `2c8fefb`.
- Eigenruhe `f74cebb`, Hebkit `841bf5d`, Leitbild `bd38b33`, Redemut `a782538`, Runtime Analyzer
  `d47bfd5`, Schlauzug `31d3f55`, and Tiefgang `2d37a06`. Each product was read at that `main`
  revision on 2026-09-27 and 2026-09-28. No product file was changed.

## Summary

| Helper                   | Baukit owner                                         | Template call site                                           | Status  |
| ------------------------ | ---------------------------------------------------- | ------------------------------------------------------------ | ------- |
| Limits policy parser     | `@baukit/data-contracts/limits`                      | `templates/{mobile/mobile,web/web}/src/limits.ts`            | shipped |
| Hydrated analytics store | `@baukit/analytics-posthog-native/storage`           | `templates/mobile/mobile/src/analytics.ts`                   | shipped |
| Route heading focus      | `@baukit/a11y-core` root                             | `templates/mobile/mobile/src/route-heading-focus.ts`         | shipped |
| Themed OIDC browser flow | `@baukit/auth-native` root and `/expo`               | `templates/mobile/__auth__/mobile/src/auth.ts`, Keycloak theme | shipped |
| Keycloak e2e helpers     | web auth template (no package)                       | `templates/web/__auth__/web/e2e/stack/keycloak.ts`           | shipped |

No helper was skipped. Two helpers cover only part of what the products copied, and the parts left
out are named under each helper.

## 1. Limits policy parser

### Observed copies

The template's `limits.ts` was identical in mobile and web. It held a hand-written validator for
six fixed sections, `LimitsPolicyError`, `LimitError(reason, field)`, and six check functions.

- Leitbild mobile and web, Redemut mobile and web, and Tiefgang mobile match the template shape.
- Runtime Analyzer `web/src/limits.ts` adds `analytics` and `downloads` sections, and its
  `checkSection` takes several keys per section.
- Schlauzug mobile and web have 12 product sections. The workshop section allows zero, and one
  cross-field check requires the rooms question time to fit inside its window.
- Eigenruhe `mobile/src/limits.ts` is version 2 with `text`, `json`, `rows`, `transport`, and
  `retention` sections, each with several keys. `transport.daily_change_budget` may be zero. Its
  `LimitError(reason, field, max)` has the message `${field}: ${reason} (max N)` and uses reasons
  such as `json_too_large` and `row_cap_*`.
- Hebkit `mobile/src/limits.ts` does not validate. `parseLimitsPolicy(policy: Limits)` checks only
  the version, and Hebkit web re-exports from mobile.

Every `limits.json` has the shape `{ $comment: string, version: int, <section>: { <key>: int } }`,
so one schema-driven parser covers all seven products. The fixed six-section parser would have
fit only three.

### Baukit owner and public contract

`@baukit/data-contracts/limits` adds:

- `LimitsPolicySchema { version, sections: Record<section, readonly key[]>, allowZero? }`.
- `LimitsPolicy<Schema>`, the parsed result typed from a `const` schema.
- `parseLimitsPolicy(value, schema)`. It requires the exact section and key set, a string
  `$comment`, the exact version, and positive safe integers, or non-negative ones for keys listed in
  `allowZero`. Messages look like `limits.text.max_characters must be a positive integer`.
- `LimitsPolicyError`, for a bad file, and `TypeError` for a bad schema, such as a section named
  `version` or an `allowZero` entry that names no key.
- `LimitError<Reason>` with `reason`, `field`, `measured`, and `allowed`. The message stays
  `Limit exceeded for ${field}: ${reason}`.
- `enforceLimit(field, reason, check)`, which turns the package's `LimitExceededError` into
  `LimitError`.

The template keeps `LIMITS_POLICY_SCHEMA`, `LIMITS_POLICY`, and one-line check functions.

### Left product-side

Schlauzug's cross-field rooms check runs after `parseLimitsPolicy`. Eigenruhe's `(max N)` message
and reason names stay in its own `LimitError` subclass or mapping. A shared parser that knew about
either would need switches.

### Failure behavior

A bad policy file throws `LimitsPolicyError` at module load, as before. An invalid count now throws
the package's `RangeError` (`measured must be a non-negative safe integer`) instead of the
template's `<field> count must be a non-negative integer`.

## 2. Hydrated analytics storage

### Observed copies

Eigenruhe, Leitbild, Schlauzug, and Tiefgang `mobile/src/analytics.ts` carry the template class
unchanged apart from the storage prefix. Hebkit `mobile/src/monitoring/storage.ts` has its own
`HydratedAnalyticsStorage` that reads keys one at a time, so one failed key does not drop the rest,
and it persists the consent key too. Redemut `mobile/src/analytics.ts:98` uses a synchronous Expo
key-value store (`ExpoAnalyticsStorage`) and needs no hydration.

### Baukit owner and public contract

- `@baukit/analytics-core` adds `analyticsStorageKeys(prefix)` and `AnalyticsStorageKeys`, the four
  keys `AnalyticsClient` reads and writes. The client now uses the same function.
- `@baukit/analytics-posthog-native/storage` adds `AsyncKeyValueStorage`,
  `HydratedAnalyticsStorageOptions { persistence, persistentKeys }`, and
  `HydratedAnalyticsStorage.load(options)`. The subpath does not import `posthog-react-native`,
  which became an optional peer.

Hebkit's per-key read won over the template's `multiGet`. Which keys persist is a caller argument,
because the products disagree on the consent key. The template keeps consent out of analytics
storage, as before.

### Failure behavior

A key that fails to read starts absent. Write-through failures, sync or async, are ignored, and the
in-memory value still changes.

## 3. Route heading focus

### Observed copies

The template's controller is `null` on native, so `useRouteHeadingFocus` did nothing there. It did
not run its web path on native.

- Leitbild `mobile/src/route-heading-focus.ts` adds an explicit native no-op hook. It behaves the
  same as the old template and is redundant.
- Redemut adds a `routeKey` argument that reruns the effect when the key changes.
- Eigenruhe falls back to `document.querySelector('main [role="heading"][aria-level="1"], main h1')`
  and swaps in a null component under `NODE_ENV=test`.
- Hebkit is the only product that focuses on native. `mobile/src/utils/accessibility-focus.ts`
  wraps `findNodeHandle` and `AccessibilityInfo.setAccessibilityFocus`, and
  `route-heading-focus.ts` schedules it one frame after the route gains focus.

### Baukit owner and public contract

`@baukit/a11y-core` exports `focusAccessibilityElement(ref)`. It resolves the view tag and calls
`setAccessibilityFocus` on native, focuses with `preventScroll` on web, and returns `false` when
there is nothing to focus. `useOverlayA11y` shares its view tag lookup.

The template's `createRouteHeadingFocusEffect` now follows Hebkit. With a controller it uses the DOM
route focus controller. Without one it calls `focusAccessibilityElement` on the next animation frame
and cancels on cleanup.

### Left product-side

Redemut's `routeKey` and Eigenruhe's DOM query fallback serve one product each.

## 4. Themed Expo OIDC browser flow

### Observed copies

Tiefgang `mobile/src/auth/themed-browser.ts` and Eigenruhe `mobile/src/auth-themed-browser.ts`
are the same 83 lines apart from import paths and the state prefix (`tg1`, `er1`). Both load the
last theme preferences from storage inside the browser flow, require both colors, and build
`<prefix>.<d|l|s>.<PRIMARY>.<SECONDARY>.<64 hex>` from 32 bytes of `expo-crypto`. Their Keycloak
decoders, `tiefgang/keycloak/themes/tiefgang/login/resources/js/theme-preferences.js` and
`eigenruhe/infra/keycloak/themes/eigenruhe/login/resources/js/theme-preferences.js`, parse the same
format and toggle `pf-v5-theme-dark`.

Hebkit `mobile/src/auth/oidc-browser.ts` is a web popup flow (`WebOidcBrowserFlow`) with its own
pending-state handling. It does not decorate state and is out of scope here.

### Baukit owner and public contract

- `@baukit/auth-native` root: `appearanceStateDecoration({ mode, primaryColor?, secondaryColor? })`
  returns `['ap1', 'd' | 'l' | 's', PRIMARY?, SECONDARY?]`. `decoratedAuthorizationState(segments,
  entropy)` joins the segments with a hex nonce and requires at least
  `AUTHORIZATION_STATE_ENTROPY_BYTES` (32) bytes. `NativeOidcClient.signIn({ stateDecoration })`
  forwards the segments through `AuthorizationRequest.stateDecoration`.
- `@baukit/auth-native/expo`: `createExpoBrowserFlow({ randomBytes })`, the AuthSession flow that
  `createExpoOidcEnvironment` already used, now public. `ExpoOidcEnvironmentOptions` accepts
  `randomBytes`.
- Keycloak theme template:
  `templates/backend/__auth__/keycloak/themes/baukit-accessible/login/resources/js/theme-preferences.js`
  reads `ap1` state from `state` or from `client_data.st` after a form post. It sets
  `--baukit-auth-primary`, `--baukit-auth-secondary`, and `--baukit-auth-on-primary`. For `d` or `l`
  it sets `data-baukit-theme` and keeps `pf-v5-theme-dark` pinned with a `MutationObserver`.

Two things differ from the product copies on purpose. The caller passes the decoration to `signIn`
instead of the flow reading storage, so the package does not know where a product keeps its theme.
Colors are optional, because the template has a mode preference and no brand colors.

### Keycloak dark-mode finding

`keycloak.v2` in 26.7.x ends `template.ftl` with an async module script. When the realm's
`darkMode` is on, it toggles `pf-v5-theme-dark` from `prefers-color-scheme` and listens for changes.
A theme script that sets the class once can be undone by that script. The Baukit decoder re-applies
the class when it changes. The real-browser check in
`templates/backend/__auth__/scripts/keycloak-theme.browser.mjs` emulates a light system preference
with a `d` state and asserts the page stays dark, before and after a failed password post. It
passed on Keycloak 26.7.0 and 26.7.1.

### Failure behavior

Without `randomBytes`, or when the entropy source or a segment fails, the flow falls back to
AuthSession's own state and sign-in continues unthemed. `appearanceStateDecoration` throws
`TypeError` when only one color is given or a color is not `#RRGGBB`. The decoder ignores any state
it does not match and leaves the page as Keycloak renders it.

### Privacy boundary

The decoration is visible to the identity provider, its logs, and anyone who sees the authorization
URL. It carries only the appearance mode and two public brand colors. It never carries user,
account, or device data. The nonce keeps the 256 bits of entropy the plain AuthSession state had.

## 5. Keycloak end-to-end helpers

### Observed copies

- Tiefgang `e2e/tests/helpers.ts:16-35` gets an `admin-cli` token and creates a user. Its sign-in
  uses `getByLabel('Username or email')`.
- Eigenruhe `e2e/tests/helpers.ts:66-106` does the same with an email address and signs in through a
  popup with label selectors (`:478-499`).
- Hebkit `web/e2e/tests/helpers.ts:509-529,790-810` does the same.
- Redemut `web/e2e/tests/helpers.ts:33-423` retries every Keycloak request, deletes an existing user
  with the same name first, and uses `#username`, `#password`, and `#kc-login`.

### Template change

The web auth template gains `web/e2e/stack/keycloak.ts` with `keycloakStack(environment)`,
`createKeycloakTestUser(request, stack)`, and `signInWithKeycloak(page, user, stack)`, plus
`e2e/stack/sign-in.spec.ts` and `e2e/playwright.stack.config.ts`. Users get random `e2e-<uuid>`
names, so there is nothing to delete first and no retry loop. The helper uses Keycloak's element IDs,
which the accessible theme keeps, because label text changes with locale. The web auth README has a
"Keycloak stack test" section and now names the correct seeded password.

This stays template code. The helpers depend on `@playwright/test` and the generated realm, and no
package in the workspace has Playwright as a dependency. No CI gate runs the stack spec, because it
needs the composed Keycloak. `EXPECTED_AUTH_WEB_FILES` does not list the new files, so `baukit
doctor` does not fail products that lack them.

## 6. MCP migration

The TypeScript MCP template has been removed. Products migrate to the Rust
remote server through the [migration guide](../migrations/mcp-stdio-to-remote.md).
`@baukit/auth-node` remains available for web OIDC test clients and other Node
clients. It is no longer a generated MCP server dependency.

## Supported runtimes

- `@baukit/data-contracts/limits` and `@baukit/analytics-core`: any ES2022 runtime.
- `@baukit/analytics-posthog-native/storage`, `@baukit/a11y-core` focus, and
  `@baukit/auth-native/expo`: React Native and Expo, plus React Native Web for the focus helper.
- `@baukit/auth-node`: Node.js 24.
- Keycloak decoder: Keycloak 26.7 with the `keycloak.v2` login theme.

## Breaks

Package exports gain only additions. The behavior changes are:

- `LimitError` gains `measured` and `allowed`, and an invalid count now throws the package message.
- `posthog-react-native` is an optional peer of `@baukit/analytics-posthog-native`.
- Template-only, recorded here because the CLI has no changelog:
  - Generated mobile apps depend on `@baukit/analytics-posthog-native`, and the CLI doctor expects
    it.
  - Native `useRouteHeadingFocus` now moves screen-reader focus.
  - The Keycloak theme lists `js/accessibility.js js/theme-preferences.js` in `scripts`. A child
    theme that sets `scripts` must list both.
  - The mobile auth template depends on `expo-crypto` and themes the login page from the current
    appearance mode.
  - Every generated MCP package depends on `@baukit/auth-node`, the CLI doctor requires it, and a
    bad `<APP>_API_URL` stops the server at startup.

## Product adoption

Deletion is deferred to each product's adoption pass. Per product:

MCP tools now follow the Rust migration guide above. Delete their old API
origin helpers with the TypeScript server after porting the tools.

- **Eigenruhe.** Replace `mobile/src/limits.ts`'s parser with a v2 schema and keep its
  `LimitError` message mapping. Delete the class in `mobile/src/analytics.ts`. Keep the DOM fallback
  in `route-heading-focus.ts` or drop it. Delete `mobile/src/auth-themed-browser.ts`, pass
  `appearanceStateDecoration` to `signIn`, and replace the `er1` decoder with the template's by
  making the theme a child of `baukit-accessible`. Replace the Keycloak user setup in
  `e2e/tests/helpers.ts`.
- **Hebkit.** Add a schema to `mobile/src/limits.ts`, which today validates nothing. Delete
  `mobile/src/monitoring/storage.ts`'s class. Delete `mobile/src/utils/accessibility-focus.ts` and
  call `focusAccessibilityElement`. Replace the Keycloak helpers in `web/e2e/tests/helpers.ts`.
- **Leitbild.** Replace the parser in `mobile/src/limits.ts` and `web/src/limits.ts`. Delete the
  class in `mobile/src/analytics.ts`. Delete the native no-op in `route-heading-focus.ts`, which
  also turns native focus on.
- **Redemut.** Replace the parser in `mobile/src/limits.ts` and `web/src/limits.ts`. Keep
  `ExpoAnalyticsStorage`. Keep `routeKey` or move it to the call site. Replace the Keycloak user
  setup in `web/e2e/tests/helpers.ts`, keeping its retry only if its stack needs it.
- **Runtime Analyzer.** Replace the parser in `web/src/limits.ts` with a schema that lists its
  multi-key sections.
- **Schlauzug.** Replace the parser in `mobile/src/limits.ts` and `web/src/limits.ts`, list the
  workshop keys in `allowZero`, and keep the rooms cross-field check. Delete the class in
  `mobile/src/analytics.ts`.
- **Tiefgang.** Replace the parser in `mobile/src/limits.ts`. Delete the class in
  `mobile/src/analytics.ts`. Delete `mobile/src/auth/themed-browser.ts`, pass
  `appearanceStateDecoration` to `signIn`, and drop the `tg1` decoder in favor of the parent theme's.
  Replace the Keycloak user setup in `e2e/tests/helpers.ts`.

## Product defects

- Tiefgang's login theme (`keycloak/themes/tiefgang/login/theme.properties`) is a child of
  `baukit-accessible` and sets `scripts=js/theme-preferences.js`. By the template's documented rule
  that list replaces the parent's, so `js/accessibility.js` does not load. This was not checked in a
  browser against Tiefgang's own stack.
- The Tiefgang and Eigenruhe decoders set `pf-v5-theme-dark` once. With realm `darkMode` on,
  `keycloak.v2`'s module script can toggle it back from the system preference. Not reproduced in
  either product.
- Hebkit and Leitbild MCP servers accept plain HTTP to loopback in production.
- Hebkit's mobile limits parser checks only the version, so a malformed `limits.json` loads.

## Tests

- `typescript/packages/data-contracts/src/limits-policy.test.ts`.
- `typescript/packages/analytics-posthog-native/src/storage.test.ts` and a new
  `analyticsStorageKeys` case in `analytics-core/src/client.test.ts`.
- `typescript/packages/a11y-core/src/native-focus.test.ts`.
- `typescript/packages/auth-native/src/authorization-state.test.ts`, new cases in `expo.test.ts`
  and `index.test.ts`.
- `typescript/packages/auth-node/src/api-origin.test.ts`.
- `templates/backend/__auth__/scripts/tests/keycloak_theme_preferences.test.mjs` and the
  appearance case in `templates/backend/__auth__/scripts/keycloak-theme.browser.mjs`.
- Template tests: `limits.test.ts` in mobile and web, and `route-heading-focus.test.ts`.

## Follow-up (2026-09-28)

Three gaps found after the six items shipped.

### Test type check

`typescript/tsconfig.test.json` type-checked every package's `*.test.ts` in one program against the
base config. It skipped `auth-node` and `data-contracts-expo-sqlite`, ignored each package's own
`lib` and `types`, and nothing ran it. On `main` it reported about 25 errors. It was not dead,
because ESLint's project list and the Dexie browser config used it, so it was replaced rather than
deleted outright.

Each of the 20 packages now has a `tsconfig.test.json` that extends its own `tsconfig.json` with
`noEmit` and covers all of `src`, tests included. Every package `test` script runs
`tsc -p tsconfig.test.json` before Vitest, so `make ts-check`, `make ts-test`, and the CI
TypeScript job run it without changes to `turbo.json`, the Makefile, or the workflow. `events`,
`localization-core`, `notifications-core`, and `sync-client` list `node` in `types` and gain
`@types/node` as a dev dependency. The Dexie test config adds the DOM lib and leaves the browser
spec to `tsconfig.browser.json`, which now extends it. ESLint resolves a file through the package
`tsconfig.json` first, so Node types do not leak into source linting.

The errors were in tests only. One was a real stale assertion: the `api-runtime` idempotency test
built an error envelope with `request_id`, which the camelCase change renamed to `requestId`. The
rest were JSON imports without `with { type: 'json' }`, an `@ts-expect-error` on the wrong line,
callbacks that returned a value where `void` was expected, optional properties set to `undefined`
under `exactOptionalPropertyTypes`, a fake timer handle typed as a number, and a dead
`RuleTester.afterAll` assignment. The fixes use no `any` and no `@ts-ignore`. The one new
`@ts-expect-error` in `node-sqlite.test.ts` checks that an untyped caller can still pass
`undefined`.

### Mobile auth coverage floors

`templates/mobile/__auth__/mobile/jest.config.cjs` had no coverage floors. It is deleted, so the
auth app uses the base config with 70% statement, branch, function, and line floors. The auth
`package.json` had also drifted: it lacked `setup` and `test:coverage`, and the generated CI calls
`test:coverage`, so an auth product's CI failed on a missing script.

With the floors on, the generated auth app measured 33% statements and 37% branches. `auth.ts`,
`local-data.ts`, `authenticated-api.ts`, and `theme-mode-control.tsx` had no tests. The base mobile
app also missed its branch floor at 67.27%. Both mobile `package.json` templates now pin
`@testing-library/react-native` 14.0.1 and `test-renderer` 1.2.0, the renderer that matches React
19.2. New template tests:

- Base: `theme-mode-control.test.tsx`, an app-preferences case in `record-store.test.ts`, and a
  `useRouteHeadingFocus` hook case in `route-heading-focus.test.ts`.
- Auth: `oidc-auth.test.tsx` (restore, discovery failure, session changes, expiry, scheduled
  refresh, sign-in with appearance, cancel, failure, sign-out), `local-data.test.tsx` (open,
  sign-out, expiry, identity mismatch, failed initialization, unmount), and
  `authenticated-api.test.ts` (401 replay, second 401, no session, global fetch).

Generated coverage is now 98.5% statements and 85.4% branches for auth, and 98.9% and 89.1% for
the base app.

### Web Keycloak stack test

`e2e/playwright.stack.config.ts` hardcoded port 5173 and reused whatever listened there. On this
machine a foreign process holds 5173, so the spec could not run without touching it. The config
now reads `E2E_WEB_PORT` (default 5173). A new `e2e/stack/global-setup.ts` calls
`allowWebOrigin` in `keycloak.ts`, which adds the origin to the web client's redirect URIs and web
origins through the admin API when the client lacks it. `E2E_KEYCLOAK_WEB_CLIENT_ID` overrides the
client ID.

Two more things failed on the way. The spec reads the subject from `/me`, so the API must list the
test origin in `<APP>__HTTP__CORS_ALLOWED_ORIGINS`. And the auth API's rate limiter connects to
Redis at startup while the product Compose file has no Redis service, so `make run` exits with
`RateLimitStoreError`. The web auth README now records the full command: Keycloak from Compose, a
disposable Redis container, `make run` with the Redis URL and CORS origin, then
`E2E_WEB_PORT=5183 corepack pnpm@11.18.0 exec playwright test --config e2e/playwright.stack.config.ts`.
The spec passed twice against the generated auth fixture on port 5183 with Keycloak 26.7.0. The
second run found the origin already registered and skipped the update.

### Breaks

All template-only, listed in the generated `CHANGELOG.md` under `[Unreleased]`:

- Auth mobile `test:coverage` now enforces the 70% floors.
- Generated mobile apps gain two dev dependencies.
- The stack test's global setup changes the development realm's web client when the port is not
  5173.

The TypeScript package changes touch tests, dev dependencies, and scripts only, so no changeset.

### Gates

- `make ts-check` (80 of 80 tasks) and `make ts-browser-test`.
- CLI `cargo fmt --check`, `clippy -D warnings`, and `cargo test -- --include-ignored`, with
  re-blessed `auth`, `combined`, `mobile`, and `strict` snapshots.
- Auth fixture (`--backend --mobile --web --auth oidc --mcp`): backend fmt and clippy; web frozen
  install, build, lint, test; mobile frozen install, `tsc --noEmit`, lint, test, `test:coverage`.
- Base fixture (`--backend --mobile --web`): mobile `test:coverage`.
- The Android native gate was not run. The emulator belongs to another task, and the new
  dependencies are Jest-only.

### Still open

- Auth web `test:coverage` fails its floors (functions 68.75%, branches 59.18%). The base web app
  passes. Same fix shape as mobile, not done here.
- The product Compose file has no Redis, so `make run` fails for every auth product until one is
  added or the README's container is started.

### F7: auth web coverage, Redis, and the fixture coverage gate

F7 closes both items under "Still open" and the Baukit CI gap that let them ship.

#### Auth web coverage floors

The generated auth web app measured 68.75% functions and 59.18% branches against the 70% floors
in `web/vitest.config.ts`. The auth overlay replaces `api.ts` but inherited the base
`api.test.ts`, which only covers `listItems`. `auth.ts` and `local-data.ts` had no tests at all.
`auth.test.ts` did test `authenticated-api.ts`, but only the replay path.

New and rewritten tests under `templates/web/__auth__/web/src/`:

- `api.test.ts` replaces the base file. It covers `listItems` and `currentUser` on valid and
  malformed bodies, and the default transport through a stubbed global `fetch` with no browser
  session.
- `auth.test.ts` keeps the PKCE authorization URL case and adds the `authClient` wiring against a
  mocked `OidcClient`: no client and no session without `window`, one lazily built client from the
  local defaults, configured issuer and client ID from `VITE_OIDC_*`, and delegation of every call.
- `authenticated-api.test.ts` takes the two replay cases from `auth.test.ts` and adds the no-session
  case and the global `fetch` fallback.
- `local-data.test.tsx` renders `useAuthenticatedLocalData` in jsdom: open on sign-in with the
  registry written to `localStorage`, sign-out, terminal expiry, explicit clear, identity mismatch,
  a corrupt registry that blocks, and a transition that settles after unmount.

The tests avoid the product name, so they need no template placeholders. Generated auth coverage
is now 91.44% statements, 83.67% branches, 90.62% functions, and 91.03% lines. No threshold changed
and no file was excluded. The remaining gaps are inherited base files (`back-or-replace.ts`,
`route-state.ts`, `accessible-dialog.tsx`), which the base app also leaves partly uncovered, and the
`canRetry === false` arm in `authenticated-api.ts`, which the runtime never reaches because it does
not call `onUnauthorized` after the one replay.

#### Redis for the auth backend

The auth API builds `RedisRateLimitStore::connect_if_enabled` at startup, and rate limiting is on by
default, so the process exits with `RateLimitStoreError` without Redis. The non-auth backend has no
rate limiter and needs no Redis. The fix follows the Keycloak service:

- `compose.yaml` gains a `redis` service (`redis:8.10.0-alpine`, the image the mobile QA stack
  already pins) under `{% if context.auth_oidc %}`, with a `redis-cli ping` healthcheck. It publishes
  on `127.0.0.1:<redis_host_port>` only, because the development Redis has no password.
- The CLI adds `redis_host_port` (6379 plus `--port-offset`) to the template context and to
  `PortConfiguration`, so the collision and overflow checks cover it. `baukit doctor` checks the
  Compose mapping for auth products, and the Makefile URL when the offset is not zero.
- `make dev` now also runs `docker compose up -d --wait redis`. With offset 0 the
  `redis://127.0.0.1/` default in `baukit-config` already matches the published port. With an
  offset, `make run` passes `<APP>__RATE_LIMIT__REDIS_URL=redis://127.0.0.1:<port>/` next to the
  existing `HTTP__PORT` and `OPS__PORT` overrides.
- `mobile/scripts/qa/docker-compose.qa.yml` declares its Redis `ports` with `!override`. Compose
  appends port lists across files, so without the tag an auth product's QA stack would also publish
  6379 and collide with the development Redis. `docker compose config` shows only 16379 for both
  the auth and the combined fixture.

Production behavior is unchanged: the API still refuses to start when it cannot reach the
configured Redis. The backend README says so, and the web auth README's stack test now runs
`docker compose up -d --wait keycloak redis` instead of a throwaway container.

Verified on the generated auth fixture: `make dev` brought Keycloak and Redis up healthy,
`make run` started the API, `/healthz` returned 200, `/readyz` reported ready, and an
unauthenticated `/me` returned 401 with `RateLimit-*` headers from the Redis-backed limiter. With
Redis stopped, the same binary exited 1 with `RateLimitStoreError(Connection refused)`. All
containers and the API were stopped afterwards.

#### Baukit fixture CI

Baukit's `generated-fixture` job ran `pnpm test` for web and mobile, while the generated product CI
runs `test:coverage`. The web step now runs `pnpm test` and then `pnpm run test:coverage`. Web
`test` also runs the service-worker script test, which `test:coverage` skips, and the Vitest suite
takes about a second, so running it twice is cheaper than copying the script into the workflow.
The mobile step replaces `pnpm test` with `pnpm run test:coverage`, which runs the same Jest suite
with coverage. `CLAUDE.md` mirrors both lines.

#### Breaks

Template-only, listed in the generated `CHANGELOG.md` under `[Unreleased]`:

- Auth products gain a `redis` Compose service on host port 6379 plus the offset, and `make dev`
  starts it. A product that already runs something on that port must stop it or pick another
  offset.
- The auth web `api.test.ts` no longer comes from the base template.

#### Gates

- CLI `cargo fmt --check`, `clippy -D warnings`, and `cargo test -- --include-ignored`, with
  re-blessed `auth`, `combined`, `mobile`, and `strict` snapshots. A new generator test checks that
  `baukit doctor` rejects an auth product whose Redis mapping ignores the port offset.
- Auth fixture (`--backend --mobile --web --auth oidc --mcp`): backend fmt, clippy, tests with
  `--include-ignored`, and `openapi_drift`; web frozen install, build, lint, test, `test:coverage`;
  mobile frozen install, `tsc --noEmit`, lint, test, `test:coverage`; MCP build, typecheck, lint,
  test, `openapi:check`, `docs:check`.
- Combined fixture (`--backend --mobile --web`): the same backend, web, and mobile gates. Web
  coverage is 84.21% statements and 74.13% branches.
- Web-only and mobile-only fixtures: install, build or `tsc --noEmit`, lint, test, `test:coverage`.
- `scripts/check-version-coherence.py`.
- The Android native gate was not run. No native dependency or app config changed.

## Follow-up 0.5.1 (2026-09-29)

### Product evidence

- Hebkit at `797fdff7` kept `web/e2e/tests/helpers.ts` because Baukit shipped the helpers only in
  the template. Its Keycloak code is `createIsolatedUser` (`:495-563`, admin token and user
  creation with a 60-second provisioning timeout and email as the username) and
  `revokeUserSessions` (`:787-821`, admin logout through `fetch`). The popup sign-in with label
  selectors and the WebKit password-grant fallback are product flow.
- Redemut copied the template helper into `web/e2e/stack/keycloak.ts`, dropped `webClientId` and
  `allowWebOrigin`, widened `signInWithKeycloak` to take `Pick<..., 'username' | 'password'>`, and
  added a 60-second wait for the login page. `web/e2e/tests/helpers.ts:5-10` and `fixtures.ts`
  import it.
- Eigenruhe `e2e/tests/helpers.ts:57-108` and Tiefgang `e2e/tests/helpers.ts:16-38` create users
  the same way, and both global setups patch a client's redirects through the admin API.

### Decision

The helpers move to a Node-only subpath, `@baukit/auth-node/keycloak-testing`. `auth-node` is the
existing Node 24 OIDC package, it already has Playwright as a dev dependency for its Keycloak
conformance script, and a subpath avoids a new npm name, which trusted publishing cannot create.

The 0.5.0 note kept this as template code because the helpers took Playwright's
`APIRequestContext` and `Page`. The package version drops both dependencies instead:

- Admin calls use `fetch`, with an injected `fetch` and a per-request timeout (default
  `DEFAULT_KEYCLOAK_REQUEST_TIMEOUT_MS`, 30 seconds). They work in a Playwright global setup, a
  spec, or a plain Node script, and Hebkit's `revokeUserSessions` already used `fetch`.
- `signInWithKeycloak` takes `KeycloakLoginPage`, a structural type with `waitForURL` and
  `locator`. A package test assigns a Playwright `Page` to it, and the generated web fixture's
  `tsc -p e2e/tsconfig.json` passes it a real `Page`.

Public API: `keycloakStack(defaults, environment)`, `createKeycloakTestUser(stack, user?,
options?)`, `revokeKeycloakUserSessions(stack, subject, options?)`,
`allowKeycloakWebOrigin(stack, origin, options?)`, `signInWithKeycloak(page, user, stack,
{ timeoutMs })`, and the types `KeycloakStack`, `KeycloakStackDefaults`, `KeycloakTestUser` (now
with `email`), `KeycloakTestUserOptions`, `KeycloakRequestOptions`, `KeycloakLoginPage`, and
`KeycloakSignInOptions`. `keycloakStack` takes the product defaults as an argument, because a
package cannot know the realm or port. `user` lets a realm that signs in by email pass its own
username, email, and password.

Errors name the step and the HTTP status, never a response body or the admin password. Created
users and added origins stay in the realm, as before.

Not moved: Hebkit's popup and WebKit password-grant flow, and the Eigenruhe and Tiefgang realm
theme and mobile client redirect patches. Those depend on each product's client and screens.

### Template change

`web/e2e/stack/keycloak.ts` now only builds `stack` from the product defaults. `global-setup.ts`
and `sign-in.spec.ts` import the helpers from the package, and the spec no longer uses the
`request` fixture. The CLI renders `@baukit/auth-node` into the web `devDependencies` for web
auth products (a `file:` path with `--baukit-path`, the release version otherwise), and
`baukit doctor` requires it there. The web auth README and the generated `CHANGELOG.md` record the
change.

### Breaks

- Template: products generated with web auth gain the `@baukit/auth-node` web dev dependency.
  `e2e/stack/keycloak.ts` no longer exports the helpers; `createKeycloakTestUser` and
  `signInWithKeycloak` take `stack` explicitly and `createKeycloakTestUser` takes no request
  context. `allowWebOrigin` is now `allowKeycloakWebOrigin`.
- `@baukit/auth-node`: additive subpath only.

### Gates

- `@baukit/auth-node` build, test (49 tests including 13 for the new subpath, and the packed
  exports check for 3 subpaths), lint, and `format:check`.
- Whole TypeScript workspace `build`, `format:check`, `lint`, `test`, and `check`: pass.
- CLI `cargo fmt --check`, `clippy --all-targets -D warnings`, and `cargo test`; only
  `cli/tests/snapshots/auth.tree` changed (web README, the three stack files, web
  `package.json`, and `CHANGELOG.md`).
- Auth fixture (`--backend --mobile --web --auth oidc --mcp`): backend fmt, clippy, tests,
  `postgres_integration` with `--include-ignored`, and `openapi_drift`; web frozen install, build
  (typechecks `e2e`), lint, test, `test:coverage`; `baukit doctor` reports healthy.
- Keycloak stack e2e under the ports lock: Compose Keycloak 26.7.0 and Redis, `make run`
  equivalent with the CORS origin, then
  `E2E_WEB_PORT=5183 pnpm exec playwright test --config e2e/playwright.stack.config.ts` twice.
  Both runs passed; the second found the origin already registered.
- Mobile and MCP fixture gates were not rerun: their generated files are byte-identical, since
  only `auth.tree` entries under `web/` and `CHANGELOG.md` changed.

## Follow-up 0.5.2 (2026-09-30)

### Product evidence

- Hebkit at `ce780c42` adopted the 0.5.1 helpers for users but kept `allowWebRedirect` in
  `web/e2e/global-setup.ts:50-110`. It reads the `post.logout.redirect.uris` client attribute,
  splits it on `##`, appends `<webUrl>/*`, and writes it back with the redirect URI.
  `allowKeycloakWebOrigin` set only `redirectUris` and `webOrigins`, so sign-out to a random e2e
  port failed without the product patch.
- Tiefgang at `7f0fd02` imports `@baukit/auth-node/keycloak-testing` from CommonJS specs
  (`e2e/package.json` has no `type`) and changed `e2e/tsconfig.json` from `"module": "node16"` to
  `"node20"` in the same commit. Its Playwright run passed with that setting.
- Tiefgang's adoption log says the plan names `analytics-core` for the hydrated analytics
  storage. The class is `HydratedAnalyticsStorage` in
  `typescript/packages/analytics-posthog-native/src/storage.ts`, exported from
  `@baukit/analytics-posthog-native/storage`. The plan's item 19 text
  (`docs/cross-product-feature-plan.md:640`) and its merge log (`:178`) name the right package;
  the nearest `analytics-core` mention is item 4's row in the sequence table (`:276`), which is
  about the scrubber. The plan was not edited.

### Decision

`allowKeycloakWebOrigin(stack, origin)` now also adds `<origin>/*` to the client's
`post.logout.redirect.uris` attribute. Keycloak stores that list as one string joined by `##`.
The helper keeps every other attribute and every existing entry, and skips the update when the
redirect URI, the web origin, and a post-logout entry are all present. A `+` entry tells Keycloak
to reuse the redirect URIs, so it counts as present. A run against Keycloak 26.7.0 with a client
that already had one post-logout URI and a PKCE attribute stored
`http://localhost:5173/*##http://localhost:5183/*` and kept the PKCE attribute; a second call made
no change.

The subpath stays ESM only, with no CommonJS build. Reasons:

- Every Baukit npm package is `"type": "module"` and ships ESM. A CommonJS build for one subpath
  would add a second compiler run, `.d.cts` types for the `require` condition, and a dual-package
  copy of the module, for a test helper.
- `@baukit/auth-node` requires Node 24, which loads ESM through `require(esm)`. Tiefgang's specs
  already run that way. Only TypeScript disagreed: `"module": "node16"` models a Node release
  that could not `require` ESM and reports TS1479 on the import, while `"node20"` and `"nodenext"`
  model the runtime the package requires. A scratch CommonJS consumer confirmed TS1479 under
  `node16` and a clean check under `node20` and `nodenext`.

So Tiefgang's `"module": "node20"` is the documented setting rather than a workaround, and it
stays. The README states the contract, and the packed-exports test now takes `--load <subpath>`
and loads `./keycloak-testing` from the packed archive with `require`, so a change that breaks
`require(esm)`, such as top-level `await`, fails the package test.

No Baukit README or platform doc placed the hydrated storage in `analytics-core`. The
`analytics-core` README told products to hydrate "an application-owned cache", which predates the
class. It now names `HydratedAnalyticsStorage` from `@baukit/analytics-posthog-native/storage` and
says `analytics-core` does not ship one.

### Breaks

None. `allowKeycloakWebOrigin` keeps its signature. A client that lacks only the post-logout
entry now gets one update where 0.5.1 made none.

### Gates

- `@baukit/auth-node` test: 53 tests (16 for the subpath, 4 of them new), and the packed-exports
  check with `--load ./keycloak-testing`.
- Keycloak 26.7.0 in Docker on a random port, driven by the built helper as described above.
- Whole TypeScript workspace `build`, `format:check`, `lint`, `test`, and `check`: pass.
- The auth web template calls `allowKeycloakWebOrigin` with an unchanged signature and its
  generated files are unchanged, so the auth fixture and the Keycloak stack e2e were not rerun.

### Product adoption

- Hebkit: delete `allowWebRedirect` and `KeycloakClientRepresentation` from
  `web/e2e/global-setup.ts` and call
  `allowKeycloakWebOrigin(keycloakStack({ url: keycloakUrl, realm: 'hebkit', webClientId: 'hebkit-app' }, {}), webUrl)`.
  Pass `{}` as the environment so `E2E_KEYCLOAK_URL` cannot replace the mapped port. The helper
  also adds the web origin, which the product patch did not.
- Tiefgang: keep `"module": "node20"` in `e2e/tsconfig.json`; no change.
