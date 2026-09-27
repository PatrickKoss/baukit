# Local notification planning evidence

Item 12 of the [cross-product feature plan](../cross-product-feature-plan.md).

## Source revisions

- Eigenruhe `f74cebb` (the working tree had unrelated changes under animations only):
  `mobile/src/features/reminders/schedule.ts:24-111` (`computeNextReminderDates`, the 14-day
  window, `afterQuietHours` at `:60-75`, and `zonedCivilTime` at `:77-111`),
  `mobile/src/features/reminders/port.ts:3-34` (`ScheduledReminder`, `ReminderSchedulerPort`,
  `FakeReminderScheduler`), `mobile/src/features/reminders/expo-adapter.ts:28-100`
  (`ExpoReminderScheduler.replace` at `:40-50`), and `mobile/src/features/reminders/scheduler.ts`
  (logical IDs `plan-reminder:<slot>`). `expo-notifications ~57.0.13`.
- Hebkit `841bf5d`: `mobile/src/features/reminders/scheduler.ts:107-237`
  (`computeNextReminderDates` at `:107-140`, `rescheduleRemindersWithAdapter` at `:142-169`, the
  shopping pair at `:171-237`, `localDateTimeToInstant` calls at `:136`, `:202`, and `:257`) and
  `mobile/src/features/reminders/notifications-adapter.ts:15-88` (iOS provisional mapping at `:21`,
  `cancelScheduledReminders` at `:61-70`, `scheduleReminder` at `:72-88`). `expo-notifications
  ~57.0.18`.
- Redemut `a782538`: `mobile/src/reminders.ts:34-75` (`decideReminderSchedule`, a device-local
  `new Date(y, m, d + offset, h, min)` loop over a 14-day horizon, IDs `practice-YYYY-MM-DD`) and
  `mobile/src/notification-adapter.ts:59-112` (`synchronizeReminders`, `disableReminders`,
  `cancelAllScheduledNotificationsAsync` at `:64` and `:111`, CALENDAR triggers at `:93`).
  `expo-notifications ~57.0.21`.
- [Study 32](../studies/32-notifications-and-timeline-playback.md), [its evidence](32-notifications-and-timeline-playback.md),
  and [item 11's evidence](47-zoned-local-time.md).
- Runtime used for the runs below: Node 24.20.0, ICU 78.3, tzdata 2026c, Vitest 4.1.10,
  `expo-notifications` 57.0.13 types.

## What the three products do today

| | Eigenruhe | Hebkit | Redemut |
|---|---|---|---|
| Instant resolution | Own `zonedCivilTime`, three correction passes | `localDateTimeToInstant` from its ICS module | Device-local `Date` constructor |
| Zone | Named, defaults to host | Named, defaults to host | Host only |
| Window | 14 civil days | 7 reminders within `cycle_days * 8` days; shopping 14 days | 14 days from device-local today |
| Ownership | `data.kind === 'plan-reminder'` | `data.kind` per reminder type | None; cancel-all |
| Identifier | Logical ID | None, Expo generates one | Logical ID |
| Diff | Cancel all owned, schedule all | Cancel all owned, schedule all | Cancel everything, schedule all |
| Results | `Promise.all`, first failure rejects | `Promise.all`, first failure rejects | `Promise.all`, first failure rejects |
| Permission | `granted` or status `granted` | Maps iOS provisional to granted | `granted` only |

None of them is stable across repeated calls. Each run cancels and reschedules every owned request,
so a failure halfway leaves a partial set, and two overlapping runs can interleave. Redemut's
cancel-all also removes any other feature's pending notifications.

## Shared vectors

`fixtures/notifications/plan-vectors-v1.json` holds 40 cases. Each case gives a zone, the gap and
fold policies, `now` as an RFC 3339 instant, `horizonDays`, optional `replaceAll`, the eligible
occurrences, and the currently scheduled entries. The expected result is either the horizon, the
`schedule` entries with instant and transition, the `keep` and `cancel` logical IDs, and the skipped
occurrences with a reason, or an error code with an optional logical ID.

| Group | Cases |
|---|---|
| Set changes | empty to full, unchanged set, unchanged set in another order, moved plus removed plus added, digest change, `replaceAll`, empty desired set, a stale past entry |
| Horizon | first and last day plus the days just outside, a one-day horizon |
| DST | Berlin spring-forward gap under `reject` and `shiftForward` (two IDs landing on one instant), Berlin fold `earlier` and `later`, a fold policy change that moves an entry, New York gap |
| Calendar | month change, 2028 leap day, year change, a zone where today is already next year |
| Travel | Berlin to New York moves every entry, Berlin to Paris keeps them, Berlin to Auckland moves today |
| Order and duplicates | same instant ordered by logical ID, duplicate desired ID, duplicate current ID |
| Invalid input | unknown zone, offset string, empty zone, 2026-02-29 inside and outside the horizon, 24:00, zero and fractional horizon, invalid clock, empty ID, ID with a space, overlong ID, empty digest, fractional current instant |

An independent Python `zoneinfo` generator produced the expected values; it is not in the
repository. The TypeScript suite checks every case, then applies the plan and plans again to prove
the second run keeps everything, then reverses both input lists to prove the order does not matter.

Instant resolution reuses item 11. A second suite feeds each of the 38 cases in
`fixtures/zoned-time/vectors-v1.json`, under all four policy pairs, through
`resolveNotificationOccurrences` with `now` two days earlier and a five-day horizon. A vector instant
must come out as a scheduled entry with the same transition, a vector gap error as a
`nonexistent_local_time` skip, and any other vector error as a thrown `NotificationPlanError` with
the same code. The core never computes an offset itself.

## Baukit owner

- `@baukit/notifications-core` (`typescript/packages/notifications-core`), new. Peer dependency
  `@baukit/localization-core ^0.4.0`, nothing else at runtime. The `./vitest` subpath imports the
  consumer's Vitest, as `@baukit/data-contracts/vitest` does.
- `@baukit/notifications-expo` (`typescript/packages/notifications-expo`), new and optional. Peer
  dependencies `@baukit/notifications-core ^0.4.0` and `expo-notifications ^57.0.13`, the lowest
  version the three products use. It imports only types from `expo-notifications`; the product
  passes the module in.

## Design decisions

- The replacement engine lives in the core, behind a four-call `NotificationPlatform<TContent>`
  port (`list`, `cancel`, `permission`, `schedule`). The Expo package implements the port and wraps
  the engine. Putting orchestration in the core means the fake and the Expo adapter run the same
  code, so the conformance suite tests one engine against two platforms.
- Ownership needs both a valid marker for the namespace and the identifier
  `baukit:<namespace>:<logicalId>`. The marker is a JSON string under `content.data.baukitNotification`
  with `version`, `namespace`, `logicalId`, `epochMilliseconds`, and `contentDigest`. One string
  value does not depend on how each platform serializes nested objects in `data`. A request with a
  copied marker but another identifier is not owned.
- Deterministic identifiers make a retry after a lost response overwrite instead of duplicate.
- The engine cancels one request at a time. A cancel that fails blocks rescheduling of that logical
  ID in the same run, so a stale request and its replacement never coexist.
- Permission is checked only when something needs scheduling. When it is missing, stale owned
  requests are still cancelled, and each item that needed scheduling fails with
  `permission_denied`. `undetermined` counts as not granted; the product decides when to prompt.
- The product passes `pendingLimit`. The engine counts every pending request on the device, since
  iOS applies its 64-request limit per app, not per feature. It schedules in instant order and
  reports the rest as `schedule_limit`.
- One replacement runs per namespace at a time, per scheduler instance. Later calls wait, and a newer
  call displaces a waiting one, which resolves as `superseded`. So a concurrent call can never
  restore an older schedule. The queue lives in the instance, not in module state, so products share
  one scheduler.
- The core plans one zone per call. The study sketch put a zone on each occurrence; none of the
  three products mixes zones in one schedule.
- Differences from the study sketch: `civilTime` strings replace `minuteOfDay`, gap policies use
  item 11's `reject` and `shiftForward` instead of `skip` and `next-valid`, instants are epoch
  milliseconds instead of `Date`, the outcome uses `kept`, `cancelled`, and `scheduled` logical ID
  lists and a `status`, and the Expo content is `NotificationContentInput` plus an optional
  `channelId` instead of flattened title, body, and data. `cancelled` matches item 20's spelling.
- No count limit. Hebkit wants the next seven reminders. It can resolve a longer horizon and take the
  first seven of `desired`, which is sorted by instant.

## Public types and errors

`@baukit/notifications-core`:

- Planning: `resolveNotificationOccurrences`, `NotificationOccurrence`,
  `NotificationOccurrenceInput`, `NotificationOccurrenceResolution`, `NotificationClock`,
  `ResolvedNotification`, `SkippedOccurrence`, `SkippedOccurrenceReason`,
  `planNotificationReplacement`, `NotificationReplacementOptions`, `NotificationReplacementPlan`,
  `PlannedNotification`.
- Ownership: `ownedNotificationIdentifier`, `encodeOwnedNotificationMarker`,
  `decodeOwnedNotificationMarker`, `isOwnedBy`, `OwnedNotificationMarker`,
  `OWNED_NOTIFICATION_DATA_KEY`, `OWNED_NOTIFICATION_MARKER_VERSION`.
- Replacement: `createOwnedNotificationScheduler`, `NotificationPlatform`, `PendingNotification`,
  `NotificationPermission`, `OwnedNotificationScheduleRequest`, `NotificationOwner`,
  `OwnedNotification`, `OwnedNotificationScheduler`, `OwnedNotificationSchedulerOptions`,
  `OwnedNotificationReplacementOutcome`, `OwnedNotificationReplacementStatus`,
  `OwnedNotificationFailure`, `OwnedNotificationFailureCode`.
- Validation: `isValidNamespace`, `isValidLogicalId`, `isValidContentDigest`,
  `MAX_NAMESPACE_LENGTH` (64), `MAX_LOGICAL_ID_LENGTH` (128), `MAX_CONTENT_DIGEST_LENGTH` (128).
  Namespaces are lowercase letters and digits separated by `.` or `-`. Logical IDs and digests are
  visible ASCII.
- Testing: `InMemoryNotificationPlatform`, `NotificationPlatformFaultState`,
  `NotificationPlatformFaults`, `StoredNotification`, and from `./vitest`
  `describeOwnedNotificationSchedulerContract`, `OwnedNotificationSchedulerHarness`, and
  `OwnedNotificationSchedulerHarnessFactory` (16 cases).
- `NotificationPlanError` with `code` and `logicalId`. Codes: `invalid_civil_date`,
  `invalid_civil_time`, `invalid_time_zone`, `invalid_horizon`, `invalid_clock`,
  `invalid_namespace`, `invalid_logical_id`, `invalid_content_digest`, `invalid_instant`,
  `invalid_pending_limit`, `duplicate_logical_id`, `reserved_data_key`. An unknown gap or fold
  policy throws `RangeError`, as in item 11.

`@baukit/notifications-expo`: `createExpoOwnedNotificationScheduler`,
`createExpoNotificationPlatform`, `ExpoNotificationsApi`, `ExpoNotificationContent`,
`ExpoOwnedNotificationSchedulerOptions`, `IOS_PENDING_NOTIFICATION_LIMIT` (64).

## Supported runtimes

The core runs wherever `@baukit/localization-core` runs: Node 24, current browsers, and Hermes with
`Intl` time-zone support. The Expo adapter targets Expo SDK 57 on iOS and Android. It loads in Node
because its runtime imports are the core only. Answers depend on the runtime's time-zone database,
as in item 11.

## Failure behavior

- Invalid planning input throws `NotificationPlanError` synchronously. `replaceOwned` rejects with it
  before touching the platform. An invalid date throws even when it falls outside the horizon, so a
  broken occurrence does not hide until its day arrives.
- Platform failures never throw. The outcome is `incomplete` with one failure per affected item:
  `list_failed` (no logical ID, nothing else runs), `cancel_failed`, `permission_denied`,
  `permission_failed`, `schedule_failed`, `schedule_limit`. Items not listed as failed succeeded.
- A retry with the same desired set converges. The conformance suite proves it after list, cancel,
  and schedule failures.

## Privacy boundary

Outcomes and errors carry codes, logical IDs, and namespace-level status only. They never include
titles, bodies, routes, or product data; a conformance case checks that the copy string never appears
in a serialized outcome. The marker holds the logical ID, instant, and digest, so products must not
put personal data in logical IDs or digests. The adapter lists every pending request but reads only
the identifier and the marker key.

## Breaks

None. Both packages are new at 0.4.0 and join the fixed release group.

## Verification

- Whole TypeScript workspace: `build`, `format:check`, `lint`, `test`, and `check` passed. The core
  runs 277 tests (plan vectors, item 11 vectors, units, conformance against the in-memory platform),
  and the Expo package 26 (conformance against a mocked module, trigger and marker shape, reserved
  key, forged and invalid markers, no cancel-all, iOS permission mapping).
- Packed packages: `pnpm pack` of `localization-core`, `notifications-core`, and
  `notifications-expo`, installed with npm into an empty project. A Node import of both packages
  resolved a Berlin gap time to `2026-03-29T01:30:00Z`, `tsc --noEmit` accepted the packed types
  against `expo-notifications` 57.0.13, and the packed `./vitest` suite passed 16 of 16 against the
  packed in-memory platform. The packed `dist` of the Expo package has no runtime import of
  `expo-notifications`.
- `scripts/check-version-coherence.py` passed with 20 packages.

## Deferred

- Real-device check. Adding the adapter to `examples/expo-sqlite-conformance` needs a new native
  module in that app's separate lockfile, a rebuilt native project, and a notification permission
  grant, which Android 13 and later ask for at runtime and `run-android.sh` does not handle. That is
  not cheap and risks `make expo-sqlite-conformance`, so a dedicated notifications device check
  belongs to the adapter's release gate, as the plan's acceptance says. Running item 11's vectors
  inside Hermes waits for the same device run.
- iOS: the 64-request limit and provisional authorization come from Apple documentation and Hebkit's
  mapping, not from a device run.

## Product adoption change

Deferred to the products, as step 4 of the item:

- Eigenruhe: replace `zonedCivilTime` and the resolution and window code in `computeNextReminderDates`
  (`schedule.ts:24-58,77-111`) with `resolveNotificationOccurrences`, keeping `afterQuietHours` as a
  product transform before it. Delete `port.ts` and replace `ExpoReminderScheduler.replace`
  (`expo-adapter.ts:40-50,73-90`) with `createExpoOwnedNotificationScheduler`. Keep category and
  channel setup (`configure`) and the permission prompt local. Existing requests carry
  `data.kind === 'plan-reminder'`; one release must cancel those once, because the new scheduler
  does not own them.
- Hebkit: replace `localDateTimeToInstant` in the two reminder planners (`scheduler.ts:136,202`) and
  `cancelScheduledReminders` plus `scheduleReminder` (`notifications-adapter.ts:61-88`) with the core
  and the Expo scheduler, one namespace per reminder kind. Take the first seven of `desired` for
  training reminders. Keep `ensureChannel`, the banner check at `:257`, and the server-push switch.
  Existing requests have generated identifiers and must be cancelled once by `data.kind`.
- Redemut: remove `cancelAllScheduledNotificationsAsync` at `notification-adapter.ts:64` and `:111`,
  replace the device-local `Date` loop in `reminders.ts:47-75` with occurrences in a named zone,
  switch from CALENDAR to the adapter's DATE triggers, and make `disableReminders` call
  `replaceOwned(owner, [])`. Weekday selection stays in the product and must use the named zone's
  civil date.

## Product defects found

- Redemut's cancel-all removes every pending notification the app owns, including any other
  feature's. It also plans in the device zone, so travel shifts reminders by the offset change.
- Hebkit schedules without an identifier, so a retry after a lost response can create duplicates,
  and its training loop may cancel requests it then fails to reschedule.
- Eigenruhe and Hebkit use `Promise.all` for cancel and schedule, so one failure rejects the whole
  run without saying which items succeeded, and overlapping runs are not serialized.
- Eigenruhe reads only `granted` and `status`, not `ios.status`. Hebkit maps provisional and
  ephemeral iOS authorization explicitly. Whether Eigenruhe skips scheduling under a provisional
  grant depends on how Expo fills `granted`, which was not checked on a device.
