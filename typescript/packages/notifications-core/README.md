# `@baukit/notifications-core`

`@baukit/notifications-core` plans local notifications and replaces only the ones a product owns.
It has no runtime dependencies besides `@baukit/localization-core` and never imports a
notification library. The product decides which occurrences are eligible and what the copy says.
This package turns them into instants, drops the ones outside the horizon, and computes what to
keep, cancel, and schedule.

## Planning

```ts
import {
  planNotificationReplacement,
  resolveNotificationOccurrences,
} from '@baukit/notifications-core';

const resolution = resolveNotificationOccurrences({
  occurrences: [
    {
      logicalId: 'plan:2026-03-29',
      civilDate: '2026-03-29',
      civilTime: '02:30',
      contentDigest: 'v3',
    },
    {
      logicalId: 'plan:2026-03-30',
      civilDate: '2026-03-30',
      civilTime: '02:30',
      contentDigest: 'v3',
    },
  ],
  timeZone: 'Europe/Berlin',
  gap: 'shiftForward',
  fold: 'earlier',
  clock: () => Date.now(),
  horizonDays: 14,
});
// resolution.desired: both entries with epochMilliseconds and transition ('gap' for the first)

const plan = planNotificationReplacement(current, resolution.desired);
// plan.keep, plan.cancel, plan.schedule, each sorted by instant, then logical ID
```

- Each occurrence resolves through `resolveZonedLocalTime` from `@baukit/localization-core`. The
  caller picks the gap and fold policies; neither has a default.
- The horizon is the civil dates from today in `timeZone` through today plus `horizonDays - 1`.
  `resolution.firstCivilDate` and `resolution.lastCivilDate` report it.
- `resolution.skipped` lists occurrences with a reason: `past` (the instant is at or before the
  clock), `outside_horizon`, or `nonexistent_local_time` (a gap under `gap: 'reject'`).
- Extra fields on an occurrence, such as the notification copy, pass through to `desired`.
- An entry is kept only when its logical ID, instant, and content digest all match. A text change
  under the same digest is kept on purpose. Change the digest, or pass `{ replaceAll: true }`, to
  replace the text.
- Invalid input throws `NotificationPlanError` with a `code` and, when it applies, the `logicalId`:
  `invalid_civil_date`, `invalid_civil_time`, `invalid_time_zone`, `invalid_horizon`,
  `invalid_clock`, `invalid_logical_id`, `invalid_content_digest`, `invalid_instant`, or
  `duplicate_logical_id`. An invalid date throws even when it falls outside the horizon. An
  unknown policy throws `RangeError`, as in `resolveZonedLocalTime`.

The same input and clock give the same result, whatever order the occurrences arrive in. The shared
vectors in `fixtures/notifications/plan-vectors-v1.json` cover DST gaps and folds, month and year
changes, travel between zones, duplicates, invalid input, changed content, and horizon boundaries.
`@baukit/notifications-core/vectors` exports `notificationPlanVectorChecks(fixture)`, the checks the
Vitest suite runs: each success case yields the expected plan, convergence once the plan is applied,
and independence from input order. `examples/expo-notifications-conformance` runs them inside
Hermes.

## Owned replacement

`createOwnedNotificationScheduler(platform, { pendingLimit })` runs a plan against a
`NotificationPlatform<TContent>` port with four calls: `list`, `cancel`, `permission`, and
`schedule`. `@baukit/notifications-expo` is the Expo implementation.

```ts
const outcome = await scheduler.replaceOwned({ namespace: 'reminders' }, desired);
// { status: 'complete' | 'incomplete' | 'superseded', kept, cancelled, scheduled, failures }
```

- A notification belongs to a namespace when it carries a valid marker for that namespace and its
  identifier is `baukit:<namespace>:<logicalId>`. Everything else is left alone, including other
  Baukit namespaces and requests with a copied marker.
- The scheduler cancels one request at a time and never cancels everything. An item whose cancel
  failed is not rescheduled in the same run.
- It checks permission only when something needs scheduling. Without permission it still cancels
  stale entries.
- It schedules in instant order and stops at `pendingLimit`, which counts every pending request on
  the device, not only this owner's.
- Failures carry a code and a logical ID, never content: `list_failed`, `cancel_failed`,
  `permission_denied`, `permission_failed`, `schedule_failed`, `schedule_limit`. Any failure makes
  the outcome `incomplete`. Calling `replaceOwned` again with the same set converges.
- One replacement runs per namespace at a time. While it runs, later calls for that namespace
  collapse to the newest. The ones that never ran resolve as `superseded`. Namespaces do not wait
  for each other. Share one scheduler per device so this works.
- An invalid namespace (lowercase letters and digits separated by `.` or `-`, at most 64
  characters) or an invalid desired set rejects with `NotificationPlanError` before the platform is
  touched.

## Conformance suite

`@baukit/notifications-core/vitest` exports `describeOwnedNotificationSchedulerContract`. Run it
against any platform implementation; it needs the consumer's Vitest installation.

```ts
import {
  InMemoryNotificationPlatform,
  createOwnedNotificationScheduler,
} from '@baukit/notifications-core';
import { describeOwnedNotificationSchedulerContract } from '@baukit/notifications-core/vitest';

describeOwnedNotificationSchedulerContract<string>((options) => {
  const platform = new InMemoryNotificationPlatform<string>();
  return {
    scheduler: createOwnedNotificationScheduler(platform, options),
    faults: platform.faults,
    content: (_logicalId, text) => text,
    addUnrelated: (identifier) => platform.addUnrelated(identifier),
    pending: () => platform.pending(),
  };
});
```

`NotificationPlatformFaultState` holds the injected faults (failed list, cancel, schedule and
permission calls, a revoked permission, a held list), so a mocked native module can reuse it.

## Boundaries

The package does not pick reminder times, quiet hours, or copy, and it does not request permission.
Products compute eligible occurrences and the content digest, ask for permission in their own UX, and
choose the pending limit for their platform. It plans one time zone per call.
