# 32. Notifications and timeline playback

## Source product files

Inspected revisions: Tiefgang `861cf0a994d5e63ec245e645023c80575759c191`, Eigenruhe `36b468d015f4aebd83a11bd662c7ff82124711fb`, Redemut `b4e8a9872595260d3f26af7d8d085aac98485e51`, and Leitbild `25eda071f0e2538b78a3ea62129a73770d506e2b`.

- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/schedule.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/schedule.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/scheduler.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/scheduler.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/port.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/expo-adapter.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/expo-adapter.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/lifecycle.tsx`
- `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/actions.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/timeline.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/scheduler.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/scheduler.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/clock.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/active-session.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/active-session.test.ts`
- `/home/patrick/projects/eigenruhe/mobile/src/audio/ports.ts`
- `/home/patrick/projects/redemut/mobile/src/reminders.ts`
- `/home/patrick/projects/redemut/mobile/src/notification-adapter.ts`
- `/home/patrick/projects/redemut/mobile/test/reminders.test.ts`
- `/home/patrick/projects/redemut/mobile/src/reference-audio.tsx`
- `/home/patrick/projects/redemut/mobile/src/spoken-practice.tsx`
- `/home/patrick/projects/redemut/mobile/src/adaptive-dialog.tsx`
- `/home/patrick/projects/tiefgang/mobile/src/features/focus/use-active-session.ts`

The Leitbild source was searched for timed media and has none. Its journal and answer timelines are visual histories. No Fitness Tracker source was required for item 32.

## Observed failure or repeated glue

Eigenruhe and Redemut both calculate stable local notification occurrences over 14 days and rebuild an Expo schedule. Redemut cancels every scheduled notification, so it can erase another feature's work. Eigenruhe safely cancels only requests marked as plan reminders. Neither product handles partial replacement or concurrent runs. Eigenruhe is the only product with a multi-cue wall-clock media runner. Redemut plays individual clips, Tiefgang projects a domain timer, and Leitbild has no timed-media playback.

## Baukit owner

An optional `@baukit/notifications-core` package should own civil-time resolution, horizon filtering, and deterministic reconciliation plans. An optional `@baukit/notifications-expo` package should own namespaced identifiers, exact ownership markers, list, owned cancellation, scheduling, and per-item outcomes. `baukit-push` keeps remote delivery. No Baukit package should own timeline playback yet.

## Public types and errors

The notification sketch names `CivilNotificationOccurrence`, `CivilTimeResolutionPolicy`, `ResolvedNotificationOccurrence`, `ExistingOwnedNotification`, `NotificationReconciliationPlan`, `NotificationOwner`, `ExpoOwnedNotification`, `OwnedNotificationFailureCode`, `OwnedNotificationReplacementOutcome`, and `ExpoOwnedNotificationAdapter`. Validation should use a typed error with codes for invalid civil date, minute, time zone, horizon, namespace, logical ID, and duplicate ID. Adapter operations report `permission_denied`, `list_failed`, `cancel_failed`, `schedule_failed`, or `schedule_limit` without copy or payload data. No timeline public type or error is accepted.

## Product-owned inputs

Products supply eligible candidates, quiet-hour adjustment, horizon length, copy, locale, content digest, category and channel setup, sound, actions, permission UX, deep links, route validation, analytics, namespace, identity policy, and retry timing. Eigenruhe keeps timelines, cue kinds, audio, keep-awake, remote controls, seek and late-cue policy, completion, and serialized product session fields.

## Concurrency, failure, privacy, and cleanup cases

Notification tests must cover two concurrent replacements for one owner, independent owners, DST gaps and folds, time-zone travel, past and duplicate candidates, content-only changes, permission revocation, OS limits, list failure, partial cancel, partial schedule, termination, retry, disable, and preservation of all unrelated requests. Cleanup touches only exact owned identifiers. Errors, logs, results, and metrics omit title, body, deep links, user IDs, product data, and raw Expo errors. A future timeline study must cover caller mutation, wall-clock jumps, monotonic time, queued ticks, adapter failure during pause and resume, seeks across cues, completion once, corrupt versioned anchors, and process restart.

## Supported runtimes

The notification core targets ES2022 browser, Node tests, and React Native without React or Expo imports. The optional Expo adapter targets the Expo Notifications versions supported by Baukit's generated iOS and Android products and needs real-device scheduling tests on both. Web notification scheduling is not included. No runtime support is claimed for a timeline package while it is deferred.

## Product adoption change

Eigenruhe can delete `mobile/src/features/reminders/schedule.ts`, `port.ts`, and most of `expo-adapter.ts`; `scheduler.ts` remains as product mapping and repository composition. Redemut can delete the civil-date loop and schedule construction mechanics from `mobile/src/reminders.ts` and replace the cancel-all and Expo scheduling block in `mobile/src/notification-adapter.ts`. Its eligibility, settings, copy, response validation, navigation, and event recording remain. No product file becomes deletable for timeline playback until a second timed-media consumer exists.

## Throwaway experiments

None. The study used source inspection only.
