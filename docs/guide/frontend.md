# Build the web and mobile frontend

At the end of this page you know which `@baukit/*` packages a generated web or Expo app already uses, what each one owns, and what stays in your product. It is for product developers working in the `web/` or `mobile/` directory of a product that `baukit new` rendered.

Every package here is a library, not a framework. Products keep their components, routes, copy and visual design. The packages hold the parts that must behave the same in every product: contrast checks, focus handling, locale resolution, consent, cache rules and the security policy.

## Design tokens and `no-raw-color`

`@baukit/ui-tokens` validates a token document, checks contrast in light and dark themes and compiles tokens to CSS variables or React Native values.

The web template runs `pnpm run tokens` before every build. That script, `templates/web/web/scripts/generate-tokens.mjs`, calls `parseTokens`, `checkContrast` and `checkSemanticContrastMatrix` with `DEFAULT_SEMANTIC_CONTRAST_REQUIREMENTS`, then writes `src/tokens.css` with `toCssVariables`. A token pair below 4.5:1 for text fails the build.

Colors stay out of components through the `no-raw-color` rule. This block is from `templates/web/web/eslint.config.js`:

```js
plugins: { '@baukit/ui-tokens': uiTokens },
rules: {
  '@baukit/ui-tokens/no-raw-color': [
    'error',
    { allowedFiles: ['src/tokens.css', 'src/styles.css'] },
  ],
},
```

The rule reports hex, `rgb()` and `hsl()` values anywhere in app code. Put a new color in the token file, not in a component. The ui-tokens README lists the rule's options and the vertical layout helpers `getUsableContentHeight` and `isShortViewport`.

## Accessibility

`@baukit/a11y-core` owns overlay focus, inert background content, announcements, reduced motion, route focus and keyboard movement through groups. React Native code imports the package root. Plain React web code imports `@baukit/a11y-core/web`, which loads no React Native code.

The web template uses it in two places:

- `src/accessible-dialog.tsx` uses `useFocusTrap`, `useInert` and `useSingleFlight` from `@baukit/a11y-core/web`.
- `src/navigation-shell.tsx` uses `createRouteFocusController` to move focus to the new page heading after a route change.

The mobile template does the same in `src/route-heading-focus.ts`. The [accessibility contract](../platform/accessibility-contract.md) states what a product must meet and names the export for each rule that has shared code.

## Navigation

`@baukit/navigation` draws a collapsible navigation shell for React DOM, from `@baukit/navigation/web`, and for React Native. Products pass routes, copy, icon renderers and tokens. The package imports no router, icon library or Tailwind.

Both templates wrap it in `src/navigation-shell.tsx`. Both also ship `src/back-or-replace.ts`, which goes back when `canGoBack()` is true and otherwise replaces the route with a fixed destination. A page opened from a deep link still has a working back button. ADR [0005](../adr/0005-shared-application-navigation.md) records why the shell is shared and the routing is not.

## Localization

`@baukit/localization-core` resolves a locale from a preference and a supported list, compares catalog keys across languages, and builds `Intl` formatters with an explicit locale. It also handles civil dates and zoned local times. Products bring the catalogs and the copy.

In the mobile template, `src/localization/i18n.ts` calls `normalizeLocalePreference` and `resolveLocale`, and `src/localization/catalogs.test.ts` fails when a catalog misses a key. The [localization contract](../platform/localization-contract.md) holds the rules for fallback, stable codes and date handling.

## Preferences

`@baukit/preferences-core` defines typed preferences, how each one updates and in which order side effects run. Products provide the definitions, a storage adapter and the effects.

The mobile template keeps theme, locale and analytics consent in `src/app-preferences.ts`, stored in a `RecordStore` from `@baukit/data-contracts`. The preferences-core README covers update modes, migration and the adapter test suite.

## Analytics and consent

`@baukit/analytics-core` is the port. It holds the consent gate, the event allowlist, the property scrubber and the buffer. `@baukit/analytics-posthog-web` and `@baukit/analytics-posthog-native` are the PostHog transports. They only see events that already passed the core.

The web template's `src/analytics.ts` builds an `AnalyticsClient` with a `NoopTransport`, so a new product sends nothing until you add a transport and an allowlist. The mobile template's `src/analytics.ts` persists identity with `HydratedAnalyticsStorage` from `@baukit/analytics-posthog-native/storage`.

Consent, identity and what may never leave the device are rules, not code choices. Read the [analytics privacy contract](../platform/analytics-privacy-contract.md) before you add an event. The [privacy guide](privacy.md) covers the server side.

## Offline web apps

`@baukit/pwa-web` decides the cache strategy for a service worker request. The decision is a pure function of method, URL, mode and destination, so tests run in Node.

The generated web app ships without a service worker. To add one, follow the steps in `templates/web/web/README.md`: add `@baukit/pwa-web`, set `capabilities.pwa = true` in `baukit.toml`, and run `pnpm build:sw`. The script `scripts/build-sw.mjs` copies `@baukit/pwa-web/worker` to `public/baukit-pwa-worker.js`. Your own `sw.js` loads it with `importScripts`. Requests with an `Authorization` header and every `/auth/session/*` call stay network only. `baukit doctor` checks the worker build when the capability is on.

## Content Security Policy with `@baukit/csp`

`@baukit/csp` builds the policy that web builds and mobile web exports serve. The package owns the directives. The product only lists its origins. ADR [0006](../adr/0006-browser-token-handling.md) section 7 records why scripts never get `'unsafe-inline'`.

In the web template the origins live in `csp/origins.ts`. Each one comes from a `VITE_*` value through `originOf`:

- `api` from `VITE_API_URL`
- `issuer` from `VITE_OIDC_ISSUER` for OIDC products
- `analytics` from `VITE_POSTHOG_HOST`
- `media` from `VITE_MEDIA_ORIGIN`
- `suiteHub` from `VITE_SUITE_HUB_API_URL`

Clerk and WorkOS products add vendor origins there too. Clerk's comes from `clerkFrontendApiOrigin`.

The build then does three things:

1. The Vite plugin in `csp/vite-plugin.ts` hashes every inline script with `inlineScripts` and `hashSource`, calls `contentSecurityPolicy` and writes `dist/csp.txt`.
2. `node csp/check-build.ts dist` runs as part of `pnpm build` and fails when an inline script has no hash.
3. `vite preview` sends the built policy with `SECURITY_HEADERS`: COOP and CORP `same-origin` and `nosniff`. The dev server sends `developmentContentSecurityPolicy`, which adds inline styles and the HMR socket.

`contentSecurityPolicy` throws on any script source that is not a `'sha256-...'` hash. `assertProductionOrigins` throws for `http:` origins, embedded credentials and loopback hosts, so a production build fails when a local `.env` leaks in.

The chart's web component requires the policy and fails to render without it. Deploy with:

```bash
helm upgrade --install app deploy/chart/baukit-app --set-file web.contentSecurityPolicy=web/dist/csp.txt
```

When your app talks to a new origin, add it to `csp/origins.ts`. Do not edit the directives. The [deploying guide](deploying.md) covers the rest of the chart.

## Parse untrusted data with `@baukit/type-guards`

Anything from the network, storage or a URL is `unknown` until a guard says otherwise. `@baukit/type-guards` has no dependencies and exports `isRecord`, `isString`, `isNonEmptyString`, `isBoolean`, `isSafeInteger`, `isNonNegativeSafeInteger`, `isStringArray` and `nullable`. This snippet is from the package README:

```ts
import { isNonEmptyString, isRecord } from '@baukit/type-guards';

function subject(claims: unknown): string | undefined {
  if (!isRecord(claims)) return undefined;
  return isNonEmptyString(claims['sub']) ? claims['sub'] : undefined;
}
```

`isRecord` accepts plain objects only, not `null` and not arrays. `baukit check` rejects `as unknown as T` in TypeScript, so a guard is the only way to narrow parsed JSON.

## Lint with `@baukit/eslint-config`

`@baukit/eslint-config` exports flat-config functions that return arrays you spread into `defineConfig`:

- `base` is `@eslint/js` recommended plus typescript-eslint `strictTypeChecked` and `stylisticTypeChecked`, with limits of 80 lines per function, 500 lines per file, depth 4, 4 parameters and complexity 15.
- `react` adds the React hooks rules.
- `reactWeb`, from `@baukit/eslint-config/react-web`, adds `eslint-plugin-jsx-a11y` for DOM files.
- `scripts` lints plain JavaScript with Node globals and no type information.
- `testRelaxedRules` turns off the function length, file length and parameter limits for test files.

Both templates build their `eslint.config.js` from these. The web template spreads `react` over `src/`, `base` over `e2e/` with `testRelaxedRules` for specs, and `scripts` over `scripts/*.mjs`. Add your own rules after the spread. Do not turn a shared rule off: `baukit check` fails on an eslint rule set to `off` or `warn` outside this package.

## Multi-step onboarding

Baukit ships no onboarding flow engine. Two pieces of shared code apply when you build one:

- Store the draft as `{ version, stepId, answers }` and read it back through type guards. Treat a missing, malformed or older-version draft as its own outcome instead of casting it to the current type.
- Write the records a completed onboarding creates in one backend transaction, with an idempotency key, so a retry after a timeout does not create them twice. The [replay-safe mutations contract](../platform/replay-safe-mutations.md) describes the key handling.

## Calendar export

Baukit has no iCalendar encoder. The [calendar export recipe](../platform/calendar-export-recipe.md) names the libraries to use and the shared zoned-time vectors to test against.

## Reference

- [`@baukit/ui-tokens`](../../typescript/packages/ui-tokens/README.md)
- [`@baukit/a11y-core`](../../typescript/packages/a11y-core/README.md)
- [`@baukit/navigation`](../../typescript/packages/navigation/README.md)
- [`@baukit/localization-core`](../../typescript/packages/localization-core/README.md)
- [`@baukit/preferences-core`](../../typescript/packages/preferences-core/README.md)
- [`@baukit/analytics-core`](../../typescript/packages/analytics-core/README.md)
- [`@baukit/analytics-posthog-web`](../../typescript/packages/analytics-posthog-web/README.md)
- [`@baukit/analytics-posthog-native`](../../typescript/packages/analytics-posthog-native/README.md)
- [`@baukit/pwa-web`](../../typescript/packages/pwa-web/README.md)
- [`@baukit/csp`](../../typescript/packages/csp/README.md)
- [`@baukit/type-guards`](../../typescript/packages/type-guards/README.md)
- [`@baukit/eslint-config`](../../typescript/packages/eslint-config/README.md)
- [Accessibility contract](../platform/accessibility-contract.md)
- [Localization contract](../platform/localization-contract.md)
- [Analytics privacy contract](../platform/analytics-privacy-contract.md)
- [Replay-safe mutations](../platform/replay-safe-mutations.md)
- [Calendar export recipe](../platform/calendar-export-recipe.md)
- [ADR 0005: shared application navigation](../adr/0005-shared-application-navigation.md)
- [ADR 0006: browser token handling](../adr/0006-browser-token-handling.md)
- [ADR 0028: TypeScript package format](../adr/0028-typescript-package-format.md)
- Web template: [`templates/web/web/README.md`](../../templates/web/web/README.md)
- Mobile template: [`templates/mobile/mobile/README.md`](../../templates/mobile/mobile/README.md)
