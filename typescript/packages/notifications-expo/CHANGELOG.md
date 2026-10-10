# @baukit/notifications-expo

## Unreleased

## 0.10.11

### Patch Changes

- Release the coordinated baukit 0.10.11 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.11

## 0.10.10

### Patch Changes

- Release the coordinated baukit 0.10.10 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.10

## 0.10.9

### Patch Changes

- Release the coordinated baukit 0.10.9 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.9

## 0.10.8

### Patch Changes

- Release the coordinated baukit 0.10.8 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.8

## 0.10.7

### Patch Changes

- Release the coordinated baukit 0.10.7 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.7

## 0.10.6

### Patch Changes

- Release the coordinated baukit 0.10.6 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.6

## 0.10.5

### Patch Changes

- Release the coordinated baukit 0.10.5 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.5

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.4

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.3

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.2

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.
- Updated dependencies
  - @baukit/notifications-core@0.10.1

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/notifications-core@0.10.0

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/notifications-core@0.9.0

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/notifications-core@0.8.0

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.
- Updated dependencies
  - @baukit/notifications-core@0.7.4

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.
- Updated dependencies
  - @baukit/notifications-core@0.7.3

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/notifications-core@0.7.2

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/notifications-core@0.7.1

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/notifications-core@0.7.0

## 0.6.0

### Minor Changes

- be675db: Raised peer floors to the versions Baukit now tests against. `@baukit/ui-tokens` takes ESLint 10 (`eslint ^10.11.0`), so products can leave ESLint 9. The Expo SDK 57 peers are `react-native ^0.86.3`, `expo-auth-session ^57.0.13`, `expo-secure-store ^57.0.4`, `expo-web-browser ^57.0.3`, `expo-sqlite ^57.0.3`, `expo-notifications ^57.0.21`, and `expo-network ^57.0.2`. `@baukit/data-contracts-dexie` needs `dexie ^4.4.6`.
- Release the coordinated baukit 0.6.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/notifications-core@0.6.0

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.
- Updated dependencies
  - @baukit/notifications-core@0.5.2

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
