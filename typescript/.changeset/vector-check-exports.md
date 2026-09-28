---
'@baukit/localization-core': minor
'@baukit/notifications-core': minor
---

Export the shared-fixture vector checks so the vectors run outside Vitest. `@baukit/localization-core/vectors` exports `zonedTimeVectorChecks(fixture)` for `fixtures/zoned-time/vectors-v1.json`, and `@baukit/notifications-core/vectors` exports `notificationPlanVectorChecks(fixture)` for `fixtures/notifications/plan-vectors-v1.json`, with the fixture types. Each check has a `label`, an `expected` value, and an `actual()` call whose result must deep-equal it. The packages' own Vitest suites now run these checks, and `examples/expo-notifications-conformance` runs them inside Hermes on an Android emulator.

No breaking changes. The root exports are unchanged.
