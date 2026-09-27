# Template code into packages evidence

Plan item 19, "Move copied template code into packages". Six helpers, one commit each.

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
| MCP API origin           | `@baukit/auth-node` root                             | `templates/mcp/mcp/src/cli.ts.jinja`                         | shipped |

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

## 6. MCP API origin validation

### Observed copies

- Eigenruhe `mcp/src/api/origin.ts` trims, accepts only an origin, and allows loopback HTTP by
  default (`allowLoopbackHttp = true`). It detects IPv4 loopback with `isIP`.
- Hebkit `mcp/src/auth.ts:40-60` allows HTTP on `localhost`, `127.0.0.1`, and `[::1]` only, always,
  including production.
- Leitbild `mcp/src/api/client.ts:170-195` accepts an `/api` path prefix and returns it with the
  origin. It also allows loopback HTTP always.
- `@baukit/auth-node` `device-flow.ts` had its own https and loopback check for provider endpoints.
- The MCP template did not validate `<APP>_API_URL`. It only stripped a trailing slash.

### Baukit owner and public contract

`@baukit/auth-node` root adds `parseApiOrigin(value, { allowLoopbackHttp?, label? })`,
`ApiOriginError` with `reason` (`invalid_url`, `insecure_scheme`, `not_an_origin`), and the
`ApiOriginOptions` and `ApiOriginErrorReason` types. Loopback means `localhost`, `::1`, or any
`127.x.x.x`. The device flow now uses the same scheme and loopback check.

The MCP template calls it once at startup with
`allowLoopbackHttp: process.env['NODE_ENV'] !== 'production'` and the setting name as `label`. A bad
value exits with `server outcome=failed api_url=<reason>`, and a stdio test asserts that output and
that the value is never printed.

### Left product-side

Leitbild's `/api` prefix. A path is exactly what `parseApiOrigin` rejects, and a second mode for it
would be a switch for one product. Leitbild can join the prefix after parsing the origin.

### Failure behavior and privacy boundary

`ApiOriginError` extends `TypeError`. Its message names the setting and the rule, never the value, so
a URL with embedded credentials does not reach logs.

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

- **Eigenruhe.** Replace `mobile/src/limits.ts`'s parser with a v2 schema and keep its
  `LimitError` message mapping. Delete the class in `mobile/src/analytics.ts`. Keep the DOM fallback
  in `route-heading-focus.ts` or drop it. Delete `mobile/src/auth-themed-browser.ts`, pass
  `appearanceStateDecoration` to `signIn`, and replace the `er1` decoder with the template's by
  making the theme a child of `baukit-accessible`. Replace the Keycloak user setup in
  `e2e/tests/helpers.ts`. Replace `mcp/src/api/origin.ts` with `parseApiOrigin`.
- **Hebkit.** Add a schema to `mobile/src/limits.ts`, which today validates nothing. Delete
  `mobile/src/monitoring/storage.ts`'s class. Delete `mobile/src/utils/accessibility-focus.ts` and
  call `focusAccessibilityElement`. Replace the Keycloak helpers in `web/e2e/tests/helpers.ts`.
  Replace `validateApiUrl` in `mcp/src/auth.ts` and decide whether production may use loopback
  HTTP.
- **Leitbild.** Replace the parser in `mobile/src/limits.ts` and `web/src/limits.ts`. Delete the
  class in `mobile/src/analytics.ts`. Delete the native no-op in `route-heading-focus.ts`, which
  also turns native focus on. Call `parseApiOrigin` in `mcp/src/api/client.ts` and append `/api`
  itself.
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
- Template tests: `limits.test.ts` in mobile and web, `route-heading-focus.test.ts`, and the MCP
  `stdio.test.ts` API URL case.
