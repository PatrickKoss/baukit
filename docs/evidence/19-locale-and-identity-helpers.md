# Evidence for item 19

## Typed catalog segments

- Source product files: `redemut/packages/localization/src/catalog-segment.ts` and its callers under
  `redemut/packages/localization/src/catalogs/`.
- Observed glue: Redemut locally maps every translated segment to the English keys and preserves
  string versus `{ one, other }` shape. Baukit previously had only runtime key comparison.
- Baukit owner: `@baukit/localization-core`.
- Public types and errors: `PluralMessage`, `CatalogMessage`, `CatalogSegment`,
  `LocalizedCatalogSegment`, `CatalogSegmentLocales`, and `defineCatalogSegment`. Invalid dynamic
  locale lists throw `RangeError`.
- Product-owned inputs: supported locale tuple, reference locale, catalogs, copy, interpolation,
  loading, namespaces, and localization adapter.
- Cases: duplicate or incomplete dynamic locale maps fail; TypeScript checks exact keys and message
  shape. The helper has no concurrency, private data, or cleanup state.
- Supported runtimes: TypeScript compilation in web, React Native, worker, and Node projects. The
  runtime function uses standard ECMAScript only.
- Adoption deletion: Redemut can delete `packages/localization/src/catalog-segment.ts` after its
  catalogs import the released Baukit types and function.

## Request locale extractor

- Source product file: `leitbild/backend/crates/leitbild-api/src/locale.rs`.
- Observed glue: the local extractor scans an undecoded query string and accepts the first supported
  header entry without quality sorting.
- Baukit owner: `baukit-http`.
- Public types and errors: `RequestLocale`, `RequestLocaleConfig`, `LocaleQueryOverride`,
  `RequestLocaleConfigError`, and `RequestLocaleRejection`. Request failures return the existing 400
  `validation_failed` envelope with bounded field text.
- Product-owned inputs: supported locales, fallback, query override parameter, and localized copy.
- Cases: decoded query priority, duplicates, unsupported explicit values, weighted header order,
  ties, wildcard and fallback behavior, malformed encodings and weights, zero quality, and query or
  header limits. Extraction is request-local, returns no submitted value in errors, and owns no
  cleanup state.
- Supported runtimes: Axum services using state directly or `FromRef` application state.
- Adoption deletion: Leitbild can delete `backend/crates/leitbild-api/src/locale.rs` after its Axum
  state supplies the released configuration.

## Display-only identity hints

- Source product files: `leitbild/web/src/identity.ts` and `leitbild/mobile/src/identity.ts`.
- Observed glue: web and native contain the same JWT payload decoder, claim precedence, and initials
  logic. Both hard-code product fallback initials.
- Baukit owner: `@baukit/api-runtime`.
- Public types and errors: `UnverifiedDisplayIdentityHints`,
  `UnverifiedDisplayIdentityFallback`, and `unverifiedDisplayIdentityHintsFromJwt`. Empty fallback
  text throws `RangeError`; malformed or unusable JWTs return the fallback.
- Product-owned inputs: access token and fallback display text. Authorization and identity ownership
  use a separately validated subject.
- Cases: claim precedence, partial names, Unicode, whitespace, malformed JSON/base64/UTF-8, non-object
  payloads, missing claims, and bounded payload input. It has no concurrency or cleanup state and
  returns no token or subject.
- Supported runtimes: browsers, React Native, workers, and Node runtimes with standard `atob`.
- Adoption deletion: Leitbild can delete both identity files after web and mobile import the released
  helper and pass their own fallback copy.

## Client UUIDv7

- Source product files: no client UUIDv7 duplicate exists. Current client ID sites include
  `redemut/mobile/src/account-services.ts` and `leitbild/mobile/src/screens/profile-data-screen.tsx`,
  which use Expo UUIDv4 and remain valid.
- Observed failure and study: `uuid@14.0.2` generated UUIDv7 in Node 24 ESM, Chromium, a dedicated
  worker, and a module service worker. Vitest 4.1.10 with Playwright 1.62.1 ran the browser cases. An
  Expo test used Expo 57.0.11, React Native 0.86.2, `expo-crypto` 57.0.1, Jest 29.7.0, and
  `jest-expo` 57.0.3. The default call failed when the test removed global `crypto.getRandomValues`.
  `v7({ random: Crypto.getRandomBytes(16) })` passed without a global polyfill. The Expo template
  already pins `expo-crypto` 57.0.1 and does not install a random-values polyfill.
- Baukit owner: no package. Products should pin `"uuid": "14.0.2"`; Baukit records the tested
  runtime contract here.
- Public types and errors: use `v7` from `uuid`. Baukit adds no UUID function or error type because
  the maintained dependency passed.
- Product-owned inputs: the exact dependency pin, Expo random-byte injection, ID use, persistence,
  and any same-millisecond ordering policy.
- Cases: generated values had the UUIDv7 version and RFC variant in Node, browser, worker,
  service-worker, and Expo tests. The dependency owns concurrency behavior. UUIDv7 exposes creation
  time and must not be treated as a secret. No cleanup state is involved.
- Supported runtimes: Node and web runtimes use `v7()`. Expo uses
  `v7({ random: Crypto.getRandomBytes(16) })` with its existing `expo-crypto` dependency.
- Adoption deletion: no current product file becomes deletable. Products may adopt UUIDv7 for new
  client-generated IDs without replacing existing UUIDv4 values.
