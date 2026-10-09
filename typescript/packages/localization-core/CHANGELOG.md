# @baukit/localization-core

## Unreleased

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.
- 8a31c75: Export the shared-fixture vector checks so the vectors run outside Vitest. `@baukit/localization-core/vectors` exports `zonedTimeVectorChecks(fixture)` for `fixtures/zoned-time/vectors-v1.json`, and `@baukit/notifications-core/vectors` exports `notificationPlanVectorChecks(fixture)` for `fixtures/notifications/plan-vectors-v1.json`, with the fixture types. Each check has a `label`, an `expected` value, and an `actual()` call whose result must deep-equal it. The packages' own Vitest suites now run these checks, and `examples/expo-notifications-conformance` runs them inside Hermes on an Android emulator.

  No breaking changes. The root exports are unchanged.

- cf3a85b: Add `resolveZonedLocalTime({ civilDate, civilTime, timeZone, gap, fold })`, which turns a civil date and local time in an IANA zone into an instant. The gap policy (`reject` or `shiftForward`) and the fold policy (`earlier` or `later`) are required and have no default. It returns `{ ok: true, epochMilliseconds, transition }` or `{ ok: false, code }` with `invalid_civil_date`, `invalid_civil_time`, `invalid_time_zone`, or `nonexistent_local_time`, and throws `RangeError` only for an unknown policy value. It reads zone rules through `Intl` and adds no dependency. New exports: `GapPolicy`, `FoldPolicy`, `LocalTimeTransition`, `ZonedLocalTime`, `ZonedLocalTimeCode`, `ZonedLocalTimeResult`, `INVALID_CIVIL_TIME_CODE`, `INVALID_TIME_ZONE_CODE`, and `NONEXISTENT_LOCAL_TIME_CODE`. Shared vectors for this function and for Rust resolvers live in `fixtures/zoned-time/vectors-v1.json`.

  No breaking changes. A product with its own `localDateTimeToInstant` can replace it with `resolveZonedLocalTime({ ..., gap: 'reject', fold: 'earlier' })`, which matches the old behavior for valid input, and must then handle the returned error code instead of a thrown `Error`.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.
- a299f89: Add typed catalog segments that enforce product locale coverage, exact reference keys, and string versus plural-message shape.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/localization-core`.
