# Localization contract

**Status:** Contract and dependency-free runtime shipped in `@baukit/localization-core`.
**Applies to:** Localized Baukit web, native, and backend-facing clients.
**Related:** [`@baukit/api-runtime`](../../typescript/packages/api-runtime/README.md).

Products own their languages, copy, namespaces, catalogs, and localization library. This contract defines the safety and completeness boundary shared across implementations.

## 1. Preference and resolution

- Persist only `system` or an allowlisted supported locale. Normalize unknown types, empty values, whitespace tricks, oversized strings, object-like keys such as `__proto__`, and unsupported tags to the configured fallback; never use an untrusted preference as an object path or dynamic import.
- Resolve a `system` preference by checking the whole device/browser language list for supported exact tags, in list order. If there is no exact match, check the list again for a normalized base language (`de-DE` or `de_DE` → `de`), also in list order. If nothing matches, use the configured fallback. Products covered by this contract configure English as that fallback.
- English is both the runtime fallback language and the per-key fallback. Missing translations must not surface raw keys when safe English text exists.
- Hydrate the local preference before rendering product UI that depends on it. Bootstrap/auth/loading copy must also have an English-safe catalog.

## 2. Persistence and synchronization

- Apply a preference locally first and persist it in the active identity-scoped store. Queue server synchronization only after local persistence succeeds.
- Add the sync field as backward-compatible and optional: old records and peers may omit it, and unknown values normalize safely. Do not make a locale rollout invalidate existing settings payloads.
- If local persistence fails, restore the previous visible preference and language resources, then report the failure. Do not show a selection that will disappear on restart.
- Locale state is user-scoped in-memory state and resets during an identity partition switch.

`@baukit/preferences-core` supplies this ordering when a product registers locale
as a `PreferenceDefinition` and creates a controller with
`createPreferenceController`. `update` publishes normalized values optimistically,
runs any `preview-with-rollback` effects, calls `PreferenceStore.patch`, restores
the previous values and rolls previews back in reverse order if that work fails,
then runs after-persistence effects. Products implement server synchronization as
an after-persistence effect; the `PreferenceScope` value `synced` is metadata and
does not enqueue synchronization by itself. A reported after-persistence effect
failure does not undo the stored value. `switchIdentity` publishes normalized
defaults before it hydrates the replacement store.

For optional synchronized fields, `decodeOptionalWireValue` and
`encodeOptionalWireValue` preserve the exact `absent`, `null`, and `value`
states. `PreferenceController.applyWireValue` treats `absent` as a no-op, while
`null` is an explicit update.

## 3. Runtime resources

- On web, update `document.documentElement.lang` whenever the resolved locale changes, including initial hydration and fallback.
- In Metro bundles, register every locale/namespace through literal, statically analyzable imports or `require` calls. Do not construct runtime resource paths.
- Require exact recursive key-set parity with English for ordinary UI namespaces. Catalog namespaces keyed by domain IDs may use a separate coverage test, but every required immutable ID must be present and no stale IDs may remain.
- Translate catalog entities by immutable IDs, never by mutable English names.
- Route dates, numbers, currencies, relative time, and user-facing units through locale-aware formatters. Avoid screen-specific formatting that silently uses the runtime default locale.

## 4. Backend codes and client errors

Every stable error, warning, and recommendation reason code emitted by the backend must have a non-empty entry in every supported locale. Tests should derive the emitted-code set from typed catalogs or fixtures so a newly emitted code fails CI until translated.

`baukit-http` requires a stable snake_case `code`, structured `details`, a request ID, and a safe public `message`. In `@baukit/api-runtime`, localized clients should resolve `ApiError.code + ApiError.details` into localized text. `ApiError.message` is the safe English fallback when a code is unknown, a catalog is unavailable, or details cannot be rendered; it is not the primary localization key.

## 5. Shared runtime

`@baukit/localization-core` implements the library-neutral behavior in this
contract:

- `normalizeLocalePreference({ value, supported, fallback })` returns `system`,
  the allowlisted spelling of an exact canonical locale match, or `fallback`.
  `fallback` must itself be a valid member of `supported`, otherwise the function
  throws `RangeError`.
- `resolveLocale({ preference, deviceLocales, supported, fallback })` applies the
  exact-match-before-base-match order above. The fallback is supplied by the
  product and is not hardcoded by the package.
- `catalogKeySet(catalog)` returns sorted recursive leaf-key paths.
  `compareCatalogKeys(reference, candidate)` returns sorted `missing` and `extra`
  paths. It compares keys, not translation values or whether strings are empty.
  Products use these functions for ordinary catalog parity, immutable catalog-ID
  coverage, and emitted-code coverage tests.
- `createLocalizedCodeResolver({ catalog, fallback })` resolves a string entry or
  calls an entry with structured details. It uses the caller-provided fallback
  when an entry is absent, blank, returns no string, or throws.
- `createNumberFormatter(locale, options?)` and
  `createDateTimeFormatter(locale, options?)` require an explicit valid locale
  supported by the corresponding `Intl` formatter. Number options cover currency
  and unit formatting. The package does not provide a relative-time formatter.

Civil-date arithmetic lives in the same package, in `civil-date.ts`. A civil date
is a `YYYY-MM-DD` calendar day with no time and no offset, which is what a diary
entry, a plan day, or a reminder date actually is. `parseCivilDate`,
`assertCivilDate`, and `civilDateValidationCode` validate the shape and reject
impossible days such as `2026-02-30`. `addCivilDays`, `civilDaysBetween`, and
`compareCivilDates` do arithmetic on the calendar, so a DST transition never adds
or drops a day. `civilDateForInstant(instant, timeZone)` and
`civilToday(timeZone)` resolve an instant to the civil day an IANA zone was on,
and `isInstantOnCivilDate` answers whether a timestamp belongs to a given local
day. The zone is always an explicit argument; `resolvedTimeZone()` is the only
way to read the host zone, and callers pass the result in. These functions use
`Intl` and nothing else.

Do not compute a user-facing day with `Date.getDate()`, a UTC slice of an ISO
string, or a millisecond offset. Each is wrong for at least one user: the first
reads the host zone rather than the user's, the second shifts the day for anyone
east or west of UTC at the wrong hour, and the third breaks on the 23- and
25-hour days that DST produces.

The package has no runtime dependencies and does not own hydration, UI state,
supported locales, or the English fallback policy.

## 6. Deliberately product-local

Supported locales, translation copy, namespace layout, catalog IDs, the settings schema, plural/unit policy, and UI controls remain product-owned. Baukit does not require i18next, ship product translations, or standardize a backend localization protocol. Library adapters, hydration providers, and preference UI remain in products until another consumer proves a shared integration boundary.

## 7. Acceptance checks

- Table-test `null`, booleans, numbers, arrays, objects, empty/whitespace strings, `__proto__`, path-like strings, very long strings, unsupported tags, mixed case, and `-`/`_` regional tags.
- Prove that exact matches across the device list take precedence over base-tag
  matches, and prove ordered fallback within each pass and English runtime/per-key
  fallback.
- Prove cold-start bootstrap copy and product UI use safe resources before and after hydration.
- Prove the web document `lang` changes with the resolved locale.
- Prove every Metro resource is reachable through a static import and every ordinary namespace has exact English key parity.
- Prove catalog coverage by immutable ID and backend emitted-code coverage in every locale.
- Prove locale-aware date, number, and unit formatting.
- Force local persistence failure and verify visible preference rollback and no sync enqueue.
- Run at least one non-English end-to-end path covering cold start, preference change, persistence across restart, a backend-coded error, and localized formatting.
