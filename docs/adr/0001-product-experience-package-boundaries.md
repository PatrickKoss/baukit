# ADR 0001: Product experience packages hold behavior, not design

## Status
Amended 2026-11-27.

## Context

A consumer product carried reusable behavior for localization, preferences, product-profile
erasure, settings, responsive layout and progress display. The part several products need is the
behavior. The visual design and the domain rules belong to that product.

Baukit already kept identity-scoped persistence in `@baukit/data-contracts`, backend conformance
helpers in `baukit-test`, and token validation in `@baukit/ui-tokens`. The new work had to extend
those homes without turning baukit into a product framework. The UI components and progress models
had one source product, so nothing showed which parts were shared and which were that product's taste.

## Decision

- `@baukit/localization-core` has no runtime dependencies. It holds locale preference
  normalization, ordered locale resolution, catalog key comparison, typed catalog segments
  (`defineCatalogSegment`), stable-code localization, civil date and time validation, zoned local
  times with explicit gap and fold policies, and explicit-locale `Intl` formatter factories. The
  `./vectors` export carries the shared zoned-time vectors. Products keep their localization
  library, supported locales, fallback, copy and catalogs.
- `@baukit/preferences-core` has no runtime dependencies. It holds preference definitions,
  normalization, wire-value codecs, optimistic persistence with rollback, ordered side effects and
  identity reset. Products keep their settings schema, storage adapters and server sync.
- `@baukit/ui-tokens` holds the token schema, validation, a contrast checker, layout-mode arithmetic
  over the product's own breakpoints, the compiler, and a `no-raw-color` ESLint rule under the
  `./eslint` export. Its one runtime dependency is `@baukit/type-guards`. ESLint is an optional
  peer. It does not depend on React, React Native or Expo.
- There is no `@baukit/ui-expo` package and no shared progress model. Both wait for a second
  consumer that can use the proposed models without importing the first product's code.
- Baukit does not ship a universal settings database, settings UI generated from backend config, or
  a gamification engine. Products own brand tokens, copy, storage schemas, migrations, screen
  composition and domain calculations.

Navigation moved out of this ADR into [ADR 0005](0005-shared-application-navigation.md).
Product-profile erasure moved into [ADR 0025](0025-erasure-and-data-subject-requests.md).
[ADR 0007](0007-toolkit-of-opt-in-units.md) states the general rule that a unit needs a second
consumer.

## Enforced by

Nothing checks these package boundaries. A test that reads each `package.json` and fails when
`@baukit/ui-tokens`, `@baukit/localization-core` or `@baukit/preferences-core` gains a React, React
Native or Expo dependency would enforce the main rule.

## Consequences

The packages stay small and usable from web, native and non-React clients. Products still write the
glue for their storage, auth provider and UI. UI and progress code stays duplicated across products
until a second consumer gives evidence for a stable contract.

## History

- 2026-08-20: accepted. The file entered this repository with baukit 0.1.0, commit `af6e2b4f`. It also kept the product-profile erasure
  sequence in `@baukit/data-contracts` and `baukit-test`. That part now lives in ADR 0025.
- 2026-10-03, commit `0157313f`: ADR 0005 replaced the navigation ownership part.
- 2026-11-27: amended to the current scope of `@baukit/ui-tokens` and `@baukit/localization-core`,
  and product names removed.
