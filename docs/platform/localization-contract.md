# Localization contract

**Status:** Contract and composition recipe; no shared localization runtime yet.
**Applies to:** Localized Baukit web, native, and backend-facing clients.
**Related:** [`@baukit/api-runtime`](../../typescript/packages/api-runtime/README.md).

Products own their languages, copy, namespaces, catalogs, and localization library. This contract defines the safety and completeness boundary shared across implementations.

## 1. Preference and resolution

- Persist only `system` or an allowlisted supported locale. Normalize unknown types, empty values, whitespace tricks, oversized strings, object-like keys such as `__proto__`, and unsupported tags to the documented safe default; never use an untrusted preference as an object path or dynamic import.
- Resolve a `system` preference from the device/browser language list in order. Match supported exact tags first, then a normalized base language (`de-DE` or `de_DE` → `de`). If nothing matches, use English.
- English is both the runtime fallback language and the per-key fallback. Missing translations must not surface raw keys when safe English text exists.
- Hydrate the local preference before rendering product UI that depends on it. Bootstrap/auth/loading copy must also have an English-safe catalog.

## 2. Persistence and synchronization

- Apply a preference locally first and persist it in the active identity-scoped store. Queue server synchronization only after local persistence succeeds.
- Add the sync field as backward-compatible and optional: old records and peers may omit it, and unknown values normalize safely. Do not make a locale rollout invalidate existing settings payloads.
- If local persistence fails, restore the previous visible preference and language resources, then report the failure. Do not show a selection that will disappear on restart.
- Locale state is user-scoped in-memory state and resets during an identity partition switch.

## 3. Runtime resources

- On web, update `document.documentElement.lang` whenever the resolved locale changes, including initial hydration and fallback.
- In Metro bundles, register every locale/namespace through literal, statically analyzable imports or `require` calls. Do not construct runtime resource paths.
- Require exact recursive key-set parity with English for ordinary UI namespaces. Catalog namespaces keyed by domain IDs may use a separate coverage test, but every required immutable ID must be present and no stale IDs may remain.
- Translate catalog entities by immutable IDs, never by mutable English names.
- Route dates, numbers, currencies, relative time, and user-facing units through locale-aware formatters. Avoid screen-specific formatting that silently uses the runtime default locale.

## 4. Backend codes and client errors

Every stable error, warning, and recommendation reason code emitted by the backend must have a non-empty entry in every supported locale. Tests should derive the emitted-code set from typed catalogs or fixtures so a newly emitted code fails CI until translated.

`baukit-http` requires a stable snake_case `code`, structured `details`, a request ID, and a safe public `message`. In `@baukit/api-runtime`, localized clients should resolve `ApiError.code + ApiError.details` into localized text. `ApiError.message` is the safe English fallback when a code is unknown, a catalog is unavailable, or details cannot be rendered; it is not the primary localization key.

## 5. Deliberately product-local

Supported locales, translation copy, namespace layout, catalog IDs, the settings schema, plural/unit policy, and UI controls remain product-owned. Baukit does not require i18next, ship product translations, or standardize a backend localization protocol. Promote helpers only after a second product converges on the same zero-dependency shape.

## 6. Acceptance checks

- Table-test `null`, booleans, numbers, arrays, objects, empty/whitespace strings, `__proto__`, path-like strings, very long strings, unsupported tags, mixed case, and `-`/`_` regional tags.
- Prove exact and base-tag resolution, ordered device fallback, and English runtime/per-key fallback.
- Prove cold-start bootstrap copy and product UI use safe resources before and after hydration.
- Prove the web document `lang` changes with the resolved locale.
- Prove every Metro resource is reachable through a static import and every ordinary namespace has exact English key parity.
- Prove catalog coverage by immutable ID and backend emitted-code coverage in every locale.
- Prove locale-aware date, number, and unit formatting.
- Force local persistence failure and verify visible preference rollback and no sync enqueue.
- Run at least one non-English end-to-end path covering cold start, preference change, persistence across restart, a backend-coded error, and localized formatting.
