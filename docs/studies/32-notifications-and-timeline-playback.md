# 32. Notifications and timeline playback

## Question and scope

Should Baukit extract local notification scheduling from Eigenruhe and Redemut, and is there a second timed-media product that justifies extracting Eigenruhe's timeline runner? The notification comparison separates civil-time conversion and finite-horizon reconciliation from reminder eligibility, quiet hours, copy, channels, actions, and deep links. The timeline search covers Tiefgang, Leitbild, and Redemut as possible second consumers. `baukit-push` remains the separate server-to-device delivery package.

## Evidence table

| Product or Baukit area | File | What it does | What varies between products |
| --- | --- | --- | --- |
| Eigenruhe civil schedule | `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/schedule.ts` and `schedule.test.ts` | Converts dated program slots and local minutes to instants in a named time zone, adjusts overnight quiet hours, filters a 14-day horizon, drops past and completed slots, and sorts stable results. | Program slots, completion states, default time, quiet hours, and 14 days are product policy. The reusable parts are civil-time resolution and horizon filtering. A real DST gap or fold is not tested. |
| Eigenruhe notification composition | `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/scheduler.ts` and `scheduler.test.ts` | Reads identity-scoped settings and the active program, supplies localized copy and data, creates `plan-reminder:<slot>` logical IDs, and asks a port to replace the set. | Eligibility, repository queries, copy, slot data, and deep link are Eigenruhe behavior. |
| Eigenruhe Expo adapter | `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/expo-adapter.ts` and `expo-adapter.test.ts` | Lists requests, cancels only those tagged `kind: plan-reminder`, checks permission, configures a category and channel, and schedules date triggers without sound. | Category actions, Android channel, copy, sound, and URL are product inputs. Its owned-only cancellation and test are the safe reference behavior. |
| Eigenruhe lifecycle and actions | `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/lifecycle.tsx`, `lifecycle.test.tsx`, `actions.ts`, and `actions.test.ts` | Handles one cold response, subscribes while mounted, reschedules on foreground, reports failures, validates remote routes, and maps a skip action to a product mutation. | App lifecycle composition can be a recipe. Routes, allowlists, mutations, and failure UI remain local. |
| Redemut schedule | `/home/patrick/projects/redemut/mobile/src/reminders.ts` and `/home/patrick/projects/redemut/mobile/test/reminders.test.ts` | Creates deterministic local-time weekday reminders for 14 days, adds one due-review nudge, selects recovery copy after a lapsed streak, and assigns stable day IDs. | Due reviews, lapse eligibility, weekday selection, the two-hour window, nudge timing, all copy, and the Today route are product policy. It uses the device time zone and has no injected zone or DST test. |
| Redemut Expo adapter | `/home/patrick/projects/redemut/mobile/src/notification-adapter.ts` and `/home/patrick/projects/redemut/mobile/test/expo-notifications-stub.ts` | Stores device-local settings, checks permission, schedules calendar triggers, validates response data, navigates, and records a learning event. | It calls `cancelAllScheduledNotificationsAsync` during synchronization and disable, so one feature can erase unrelated schedules. Settings identity and device ownership are unresolved. |
| Notification failure evidence | `/home/patrick/projects/eigenruhe/mobile/src/features/reminders/expo-adapter.test.ts` and `/home/patrick/projects/redemut/mobile/test/reminders.test.ts` | Eigenruhe proves unrelated scheduled requests survive replacement. Redemut proves that denial produces no schedule and corrupt settings fall back. | Neither product tests a list, cancel, or schedule failure halfway through replacement, an operating-system schedule limit, permission revocation after listing, or concurrent replacement. |
| Baukit push delivery | `rust/crates/baukit-push/README.md` | Sends remote notifications, tracks Expo tickets and receipts, and maps provider failures. | It does not calculate local civil occurrences or own client-scheduled identifiers. Local scheduling must not be added to the Rust push package. |
| Eigenruhe timeline model | `/home/patrick/projects/eigenruhe/mobile/src/audio/timeline.ts`, `/home/patrick/projects/eigenruhe/mobile/src/content/types.ts`, and `/home/patrick/projects/eigenruhe/mobile/src/audio/spike-timeline.ts` | Represents absolute-position steps and markers, looks up the current step, and compiles voice, silence, bell, and product cue types. | Types are readonly in TypeScript but the runner keeps the caller object without freezing or copying it. Cue kinds, clips, marker meaning, and content compilation are product-owned. |
| Eigenruhe wall-clock runner | `/home/patrick/projects/eigenruhe/mobile/src/audio/scheduler.ts` and `scheduler.test.ts` | Serializes ticks, starts current voice at a late offset, suppresses a bell after a one-second grace, avoids replay, pauses, resumes, seeks, emits completion once, and reports audio startup failure. | It calls `Date.now()`, so it is an epoch wall-clock runner rather than a monotonic-clock core. It directly owns audio ducking, bells, voice, and keep-awake. Completion always stops the runner. |
| Eigenruhe anchors | `/home/patrick/projects/eigenruhe/mobile/src/audio/clock.ts`, `/home/patrick/projects/eigenruhe/mobile/src/audio/active-session.ts`, and `active-session.test.ts` | Stores epoch start, accumulated pause, optional pause instant, timeline ID, and expected end. It rebuilds active or completed-while-away state and clears corrupt JSON. | Epoch anchors are useful for process recovery, but elapsed in-process time is exposed to clock changes. Practice IDs, process tokens, sleep timer, storage key, and away-completion policy are product inputs. |
| Eigenruhe adapters | `/home/patrick/projects/eigenruhe/mobile/src/audio/ports.ts` and player files under `/home/patrick/projects/eigenruhe/mobile/src/features/player/` | Defines audio, keep-awake, lock-screen metadata, mixer, and remote-command operations. | These are adapter and product concerns under the plan. The current runner still calls part of this port directly. |
| Redemut media search | `/home/patrick/projects/redemut/mobile/src/reference-audio.tsx`, `/home/patrick/projects/redemut/mobile/src/spoken-practice.tsx`, and `/home/patrick/projects/redemut/mobile/src/adaptive-dialog.tsx` | Plays individual reference or recorded clips and records speech. Players seek a clip to zero and let Expo Audio own its position. | There is no immutable multi-cue timeline, late-tick rule, serialized anchor, or pause and resume coordinator. Redemut is not a second timeline consumer. |
| Tiefgang and Leitbild search | `/home/patrick/projects/tiefgang/mobile/src/features/focus/use-active-session.ts` and the checked-out Leitbild web and mobile source | Tiefgang ticks once per second to project its product focus-session state and catch up domain boundaries. Leitbild has journal timelines but no timed-media runner. | A domain timer and visual history are not timed-media playback. Neither can adopt Eigenruhe's runner interface. |

No second timed-media product exists in the four local product repositories. Redemut is the closest because it uses Expo Audio, but its clips have no caller-owned cue timeline or recovery anchor.

## Candidate interface or contract sketch

The two notification implementations support a pure core and an optional Expo adapter. Eligibility runs before these functions. Quiet-hour adjustment also remains a product transform that changes or drops candidate civil times before resolution.

```ts
interface CivilNotificationOccurrence {
  readonly logicalId: string;
  readonly civilDate: string;
  readonly minuteOfDay: number;
  readonly timeZone: string;
}

interface CivilTimeResolutionPolicy {
  readonly gap: "skip" | "next-valid";
  readonly fold: "earlier" | "later";
}

interface ResolvedNotificationOccurrence {
  readonly logicalId: string;
  readonly instant: Date;
}

function resolveCivilNotificationOccurrence(
  occurrence: CivilNotificationOccurrence,
  policy: CivilTimeResolutionPolicy,
): ResolvedNotificationOccurrence | null;

function occurrencesWithinHorizon(
  occurrences: readonly ResolvedNotificationOccurrence[],
  now: Date,
  days: number,
  timeZone: string,
): readonly ResolvedNotificationOccurrence[];

interface ExistingOwnedNotification {
  readonly logicalId: string;
  readonly platformId: string;
  readonly instant: Date;
}

interface NotificationReconciliationPlan {
  readonly keep: readonly ExistingOwnedNotification[];
  readonly cancel: readonly ExistingOwnedNotification[];
  readonly schedule: readonly ResolvedNotificationOccurrence[];
}

function planNotificationReconciliation(
  current: readonly ExistingOwnedNotification[],
  desired: readonly ResolvedNotificationOccurrence[],
): NotificationReconciliationPlan;
```

The core validates civil dates, minute range, horizon, time zone, duplicate logical IDs, and non-finite instants. Shared vectors must pin gap, fold, month and year boundaries, time-zone travel after reconciliation, past occurrences, stable ordering, and an empty horizon. Equality uses logical ID and instant. Payload changes remain visible to the adapter through a caller digest or an explicit replace flag; the core must not compare copy or deep-link data.

```ts
interface NotificationOwner {
  readonly namespace: string;
}

interface ExpoOwnedNotification {
  readonly logicalId: string;
  readonly instant: Date;
  readonly title: string;
  readonly body: string;
  readonly data: Readonly<Record<string, string>>;
  readonly categoryId?: string;
  readonly channelId?: string;
  readonly contentDigest: string;
}

type OwnedNotificationFailureCode =
  | "permission_denied"
  | "list_failed"
  | "cancel_failed"
  | "schedule_failed"
  | "schedule_limit";

interface OwnedNotificationReplacementOutcome {
  readonly keptLogicalIds: readonly string[];
  readonly canceledLogicalIds: readonly string[];
  readonly scheduled: Readonly<Record<string, string>>;
  readonly failures: readonly {
    readonly code: OwnedNotificationFailureCode;
    readonly logicalId?: string;
  }[];
}

interface ExpoOwnedNotificationAdapter {
  replaceOwned(
    owner: NotificationOwner,
    desired: readonly ExpoOwnedNotification[],
  ): Promise<OwnedNotificationReplacementOutcome>;
}
```

The adapter validates the namespace and logical IDs, assigns a namespaced platform identifier, and adds a package ownership marker to content data. It lists all scheduled requests but cancels only requests with that exact namespace marker and identifier prefix. It never calls Expo's cancel-all operation. The product configures categories and channels before replacement. The adapter reports per-item outcomes so a cancel or schedule failure cannot be mistaken for a complete replacement. Errors and outcomes omit titles, bodies, routes, user IDs, and provider payloads.

The evidence does not support a timeline interface yet. Before another study sketches one, a second product must supply an immutable cue list, monotonic elapsed-time port, late-tick rules, pause, resume, seek, completion, and process recovery. Eigenruhe must first split cue emission from audio and keep-awake, inject both monotonic and epoch clocks, and make late-cue policy an input.

## Required-case coverage

| Required case | Coverage today | Required package behavior or missing proof |
| --- | --- | --- |
| Civil-time occurrence | Eigenruhe resolves a named zone and proves 08:00 stays local across the spring offset change. Redemut uses local `Date` construction. | Shared vectors must cover an actual nonexistent time, both instances of a repeated time, invalid dates, invalid zones, and travel between zones. |
| Finite-horizon reconciliation | Both products default to 14 days and create stable logical IDs. Eigenruhe's port replaces the desired set; Redemut cancels all before rebuilding. | The core must plan keep, cancel, and schedule sets deterministically. Test unchanged, moved, removed, added, duplicate, empty, and content-only changed requests. |
| Reminder eligibility | Eigenruhe filters active-program slots and terminal states. Redemut uses weekday, due-review, and lapsed-streak rules. | Keep all eligibility callbacks and product records outside the core. The core accepts already eligible candidates. |
| Quiet-hour policy | Eigenruhe moves candidates out of same-day or overnight quiet periods. Redemut has no quiet-hour policy. | Keep it product-owned. Test integration by passing adjusted candidates into the pure resolver; do not put quiet hours in the package API. |
| Copy | Both products construct titles and bodies locally. | The adapter treats copy as opaque input and never logs or returns it in failures. Locale changes trigger a product-requested reconciliation. |
| Channels and actions | Eigenruhe configures one Android channel and two category actions. Redemut uses neither. | Keep creation, importance, sound, vibration, and actions in product code. The adapter accepts identifiers only. |
| Deep links | Eigenruhe validates remote routes by notification kind and maps local actions. Redemut validates a Today payload and records an event. | Keep route allowlists, navigation, mutations, and analytics local. Ownership metadata uses reserved keys and cannot overwrite product data. |
| Owned cancellation | Eigenruhe lists and cancels only `plan-reminder` requests, with a test that preserves a session bell. Redemut calls cancel-all during sync and disable. | The Expo adapter must have a conformance test proving two Baukit namespaces and one unrelated Expo request survive replacement of one owner. Disable means `replaceOwned(owner, [])`. |
| Notification concurrency | Neither product serializes two replacements. | Serialize per owner, coalesce to the newest desired set, and fence stale list and schedule completions. Different owners may reconcile independently. |
| Notification failure and cleanup | Eigenruhe lifecycle reports top-level failures. Neither adapter covers partial cancellation or scheduling. | Test permission revocation, list failure, one cancel failure, one schedule failure, OS limits, app termination, retry, and cleanup of only successfully replaced owned identifiers. |
| Immutable caller-owned timeline | Eigenruhe uses readonly types but retains the caller's arrays and objects. No mutation test exists. | A future core must either validate and copy once or require a frozen structure and reject mutation. A second product must prove the choice. |
| Monotonic clock | Eigenruhe's interval passes `Date.now()` to an epoch clock. | No product covers a monotonic elapsed clock or wall-clock jumps. This blocks extraction. Serialized epoch anchors may rebuild a monotonic session, but must not drive in-process elapsed time directly. |
| Deterministic late ticks | Eigenruhe starts a late voice at its offset, drops a bell after a four-tick grace, marks missed steps, serializes tick promises, and tests a 90-second suspension. | Cue grace and catch-up policy are hard-coded and coupled to bell and voice types. A second product must need the same policy through neutral cue events. |
| Pause and resume | Eigenruhe stops voice, removes the active step from the fired set, pauses its anchor, then restarts at the current offset. | Covered in one product only. Failure after clock resume but before adapter resume needs an explicit state and retry rule. |
| Seek | Eigenruhe clamps seek to one millisecond before the end and replays the target step. | The clamp and replay choice are product policy. Backward and forward seeks across many instantaneous cues are not tested. |
| Completion | Eigenruhe emits completion once, disables keep-awake, and stops. `ActiveSessionStore` reports completion while away. | Completion policy remains product-owned. A future core may emit a neutral boundary event but cannot decide persistence, audio, celebration, or next content. |
| Serialized anchors | Eigenruhe stores epoch start, paused total, optional paused instant, end instant, and timeline ID; tests cover active, paused, corrupt, and completed-away records. | The format mixes product and clock fields and has no schema version. A future core needs versioned validation, wall-clock rollback handling, and privacy-safe corrupt-data errors. |
| Audio, keep-awake, remote controls, cue types, and completion policy | Eigenruhe's `AudioPort` separates device calls, but `TimelineRunner` invokes voice, bell, duck, and keep-alive operations itself. Redemut has independent clip players. | Move these behind product event handling before reconsidering extraction. They do not belong in a neutral timeline core. |

Notification implementation needs unit vectors, fake-adapter conformance, packed-package export checks, and real Expo tests on supported iOS and Android versions. Timeline work needs no Baukit implementation or compatibility promise in this item.

## Decision

Decision: implement notification packages only. Add a framework-free `@baukit/notifications-core` for civil-time resolution, horizon filtering, and deterministic reconciliation planning, plus an optional `@baukit/notifications-expo` adapter that replaces one validated owner namespace and reports per-item outcomes. Redemut and Eigenruhe prove this boundary, and Eigenruhe proves safe ownership. Defer timeline extraction until a second timed-media product exists and Eigenruhe replaces `Date.now()` elapsed timing with injected monotonic time while moving cue and device policy out of the runner. The smallest next step is to write shared civil-time vectors and an Expo ownership conformance fake, then adopt the packages in both notification implementations. No timeline package should be created now.

## What stays product-owned

- Reminder eligibility, weekdays, source records, terminal states, quiet-hour policy, default reminder time, horizon length, reschedule triggers, and whether settings belong to a device or identity.
- Titles, bodies, localization, permission-prompt timing and copy, category actions, Android channels, sound, vibration, badges, deep-link allowlists, navigation, mutations, analytics, and learning events.
- Owner namespace selection, migration from existing identifiers, retry timing after partial replacement, and the product response to schedule limits or denied permission.
- Timeline definitions, content compilation, cue kinds, clips, audio, mixer ducking, bells, keep-awake, lock-screen metadata, remote commands, sleep timer, completion policy, celebrations, and active-session storage keys.
- Product decisions for late cues, seek clamping, replay after seek, completion while away, corrupted anchors, and fallback when an audio asset is unavailable.
