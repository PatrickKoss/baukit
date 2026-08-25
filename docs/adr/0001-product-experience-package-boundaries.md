# ADR 0001: Product experience package boundaries

## Status

Accepted, 2026-08-20.

## Context

Fitness Tracker contains reusable behavior for localization, preferences,
product-profile erasure, settings, responsive layout, and progress presentation.
The useful boundary is the behavior that several products need, not Fitness
Tracker's visual design or domain rules.

Baukit already keeps identity-scoped persistence in `@baukit/data-contracts`,
backend conformance helpers in `baukit-test`, and token validation and compilation
in `@baukit/ui-tokens`. The Product Experience work should extend those boundaries
without turning Baukit into a product framework.

The Expo components and progress models have only one active source application.
Fitness Tracker's Wave 3 redesign is still changing their call sites. A second
consumer is needed before Baukit can distinguish shared behavior from Fitness
Tracker-specific choices.

## Decision

- Add `@baukit/localization-core` for dependency-free locale preference
  normalization, ordered locale resolution, recursive catalog key comparison,
  localized code resolution, and explicit-locale number and date-time formatting.
  The package contains behavior only. Products keep their localization library,
  supported locales, fallback choice, copy, catalogs, and provider composition.
- Add `@baukit/preferences-core` for dependency-free preference definitions,
  normalization, optional wire-value codecs, optimistic persistence with
  rollback, ordered side effects, and identity reset. The package contains
  behavior only. Products keep their settings schema, database columns, storage
  adapters, server synchronization, and migrations.
- Keep the `eraseProductProfile` client sequence and
  `ScopedPersistenceLifecycle.eraseActivePartition()` in
  `@baukit/data-contracts`. Keep `ProductProfileErasureAdapter`,
  `check_product_profile_erasure_conformance`, and the PostgreSQL foreign-key
  audit in `baukit-test`. Products supply the HTTP and idempotency protocol,
  schema-specific deletion, and external-processor adapters.
- Defer `@baukit/ui-expo` and the progress presentation contract until Fitness
  Tracker Wave 3 has settled and a second consumer, Leitbild or OpenDialog, has
  proved the proposed component and progress models without Fitness imports.
- Keep `@baukit/ui-tokens` limited to token schema, validation, and compilation.
  It will not depend on React, React Native, or Expo.
- Do not add a universal settings database, generate settings UI from backend
  configuration, or standardize a gamification engine.
- Products own brand tokens, copy, catalogs, storage schemas, migrations, screen
  composition, and domain calculations. Baukit shares tested behavior, not a
  visual preset.

## Consequences

The first Product Experience packages stay small and usable by web, native, and
non-React clients. Adding them does not move product data ownership or migration
authority into Baukit.

Products still write integration code for their storage, authentication provider,
external processors, and UI. This is deliberate because those choices affect data
retention, permissions, and product meaning.

UI and progress code will remain duplicated for a while. Baukit will accept that
cost until two products supply evidence for a stable contract. Any later Expo
package will consume application-provided semantic tokens and copy, while
`@baukit/ui-tokens` remains independent of UI runtimes.
