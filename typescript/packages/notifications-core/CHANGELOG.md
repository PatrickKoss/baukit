# @baukit/notifications-core

## Unreleased

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.
- Updated dependencies
  - @baukit/localization-core@0.10.2

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.
- Updated dependencies
  - @baukit/localization-core@0.10.1

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/localization-core@0.10.0

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/localization-core@0.9.0

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/localization-core@0.8.0

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.
- Updated dependencies
  - @baukit/localization-core@0.7.4

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.
- Updated dependencies
  - @baukit/localization-core@0.7.3

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/localization-core@0.7.2

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/localization-core@0.7.1

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/localization-core@0.7.0

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/localization-core@0.6.0

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.
- Updated dependencies
  - @baukit/localization-core@0.5.2

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.
- Updated dependencies
  - @baukit/localization-core@0.5.1

## 0.5.0

### Minor Changes

- ea5c1cf: Add `@baukit/notifications-core`, which plans local notifications without a notification library. `resolveNotificationOccurrences({ occurrences, timeZone, gap, fold, clock, horizonDays })` resolves each `{ logicalId, civilDate, civilTime, contentDigest }` through `resolveZonedLocalTime`, keeps the ones inside the horizon, and reports the rest as `past`, `outside_horizon`, or `nonexistent_local_time`. `planNotificationReplacement(current, desired, { replaceAll })` returns deterministic `keep`, `cancel`, and `schedule` sets by logical ID, instant, and content digest. `createOwnedNotificationScheduler(platform, { pendingLimit })` runs that plan against a `NotificationPlatform` port for one namespace at a time, touches only requests that carry the namespace's marker and the `baukit:<namespace>:<logicalId>` identifier, and returns `{ status, kept, cancelled, scheduled, failures }` with content-free failure codes. Invalid input throws `NotificationPlanError`. `InMemoryNotificationPlatform`, `NotificationPlatformFaultState`, and the `@baukit/notifications-core/vitest` conformance suite let products test their own platform code. Shared vectors live in `fixtures/notifications/plan-vectors-v1.json`.

  Add `@baukit/notifications-expo`, the optional `expo-notifications` adapter. `createExpoOwnedNotificationScheduler(Notifications, { pendingLimit })` schedules DATE triggers with the marker under `content.data.baukitNotification`, never calls `cancelAllScheduledNotificationsAsync`, and maps iOS provisional and ephemeral authorization to granted. `expo-notifications` `^57.0.13` is a peer dependency and the package imports only its types.

  No breaking changes. Both packages are new.

- Release the coordinated baukit 0.5.0 train.
- 8a31c75: Export the shared-fixture vector checks so the vectors run outside Vitest. `@baukit/localization-core/vectors` exports `zonedTimeVectorChecks(fixture)` for `fixtures/zoned-time/vectors-v1.json`, and `@baukit/notifications-core/vectors` exports `notificationPlanVectorChecks(fixture)` for `fixtures/notifications/plan-vectors-v1.json`, with the fixture types. Each check has a `label`, an `expected` value, and an `actual()` call whose result must deep-equal it. The packages' own Vitest suites now run these checks, and `examples/expo-notifications-conformance` runs them inside Hermes on an Android emulator.

  No breaking changes. The root exports are unchanged.

### Patch Changes

- Updated dependencies [8d268e1]
- Updated dependencies
- Updated dependencies [8a31c75]
- Updated dependencies [cf3a85b]
  - @baukit/localization-core@0.5.0
