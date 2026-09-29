# @baukit/notifications-expo

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.
- Updated dependencies
  - @baukit/notifications-core@0.5.1

## 0.5.0

### Minor Changes

- ea5c1cf: Add `@baukit/notifications-core`, which plans local notifications without a notification library. `resolveNotificationOccurrences({ occurrences, timeZone, gap, fold, clock, horizonDays })` resolves each `{ logicalId, civilDate, civilTime, contentDigest }` through `resolveZonedLocalTime`, keeps the ones inside the horizon, and reports the rest as `past`, `outside_horizon`, or `nonexistent_local_time`. `planNotificationReplacement(current, desired, { replaceAll })` returns deterministic `keep`, `cancel`, and `schedule` sets by logical ID, instant, and content digest. `createOwnedNotificationScheduler(platform, { pendingLimit })` runs that plan against a `NotificationPlatform` port for one namespace at a time, touches only requests that carry the namespace's marker and the `baukit:<namespace>:<logicalId>` identifier, and returns `{ status, kept, cancelled, scheduled, failures }` with content-free failure codes. Invalid input throws `NotificationPlanError`. `InMemoryNotificationPlatform`, `NotificationPlatformFaultState`, and the `@baukit/notifications-core/vitest` conformance suite let products test their own platform code. Shared vectors live in `fixtures/notifications/plan-vectors-v1.json`.

  Add `@baukit/notifications-expo`, the optional `expo-notifications` adapter. `createExpoOwnedNotificationScheduler(Notifications, { pendingLimit })` schedules DATE triggers with the marker under `content.data.baukitNotification`, never calls `cancelAllScheduledNotificationsAsync`, and maps iOS provisional and ephemeral authorization to granted. `expo-notifications` `^57.0.13` is a peer dependency and the package imports only its types.

  No breaking changes. Both packages are new.

- Release the coordinated baukit 0.5.0 train.

### Patch Changes

- Updated dependencies [ea5c1cf]
- Updated dependencies
- Updated dependencies [8a31c75]
  - @baukit/notifications-core@0.5.0
