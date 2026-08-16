---
name: baukit-localization
description: Add, extend, or review localization in a Baukit web or native product while preserving safe preference resolution, local-first persistence, Metro resource loading, catalog parity, backend-code coverage, accessible document language, and localized formatting. Use for adding locales, translating catalogs or stable API/reason codes, changing language settings, or diagnosing locale fallback and persistence behavior.
---

# Localize a Baukit product

Read `<baukit-repo>/docs/platform/localization-contract.md` before changing product code. Keep the product's localization library, languages, copy, namespace layout, settings schema, and UI product-owned.

## Inventory the contract surface

1. Find the locale preference type, device/browser resolution, bootstrap resources, persistence setting, sync DTO, web root, Metro resource registry, formatter helpers, catalog IDs, and backend-emitted code sets.
2. Define the supported locale allowlist and English fallback. Normalize only `system` or allowlisted tags; test hostile non-string and path-like input.
3. Resolve regional tags to an exact supported tag or supported base language without constructing dynamic resource paths.

## Implement one locale end to end

1. Add literal static imports for every Metro namespace.
2. Match English's complete recursive key set for ordinary namespaces. For domain catalogs, match the immutable ID set instead of translating English names.
3. Cover every stable API error, warning, and recommendation reason code with non-empty localized text.
4. Route dates, numbers, currencies, relative time, and units through locale-aware formatters.
5. Set `document.documentElement.lang` on web whenever the resolved locale changes.
6. Persist the preference to the active local identity partition before enqueueing its backward-compatible optional sync field. Roll visible state/resources back if persistence fails.

For `@baukit/api-runtime` errors, use `ApiError.code` and structured `ApiError.details` as the localization input. Use `ApiError.message` only as safe fallback text; never parse it or treat it as a stable key.

## Verify

Add focused tests for preference normalization, exact/base regional resolution, English per-key fallback, bootstrap copy, key-set and immutable-ID parity, emitted-code coverage, web `lang`, static Metro resources, formatting, persistence rollback, and no sync enqueue on failed persistence. Run at least one non-English E2E path through cold start, selection, restart, a coded backend error, and localized formatting.

Run the product's required format, typecheck, lint, unit, and E2E commands. Do not add a shared localization runtime or copy another product's languages, copy, IDs, or settings schema.
