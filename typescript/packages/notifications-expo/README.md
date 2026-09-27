# `@baukit/notifications-expo`

The `expo-notifications` implementation of the `@baukit/notifications-core` platform port. It
schedules, lists, and cancels only the notifications that carry its owner's marker, and it never
calls `cancelAllScheduledNotificationsAsync`.

```ts
import { resolveNotificationOccurrences } from '@baukit/notifications-core';
import {
  createExpoOwnedNotificationScheduler,
  IOS_PENDING_NOTIFICATION_LIMIT,
} from '@baukit/notifications-expo';
import * as Notifications from 'expo-notifications';

const scheduler = createExpoOwnedNotificationScheduler(Notifications, {
  pendingLimit: IOS_PENDING_NOTIFICATION_LIMIT,
});

const { desired } = resolveNotificationOccurrences({ occurrences, timeZone, gap, fold, clock, horizonDays });
const outcome = await scheduler.replaceOwned(
  { namespace: 'reminders' },
  desired.map((entry) => ({
    ...entry,
    content: { content: { title: entry.title, body: entry.body }, channelId: 'reminders' },
  })),
);
```

- Create one scheduler per app and share it. Replacements for one namespace run one at a time on
  that instance.
- Each request gets the identifier `baukit:<namespace>:<logicalId>` and a DATE trigger at the
  planned instant. `channelId` is optional and Android only.
- The adapter writes its marker under the `baukitNotification` key in `content.data`, next to the
  product's own data. Product data that already uses that key rejects with
  `NotificationPlanError('reserved_data_key')`.
- Permission counts as granted when `granted` is true or the iOS status is authorized, provisional,
  or ephemeral. `undetermined` counts as not granted, so the product asks first.
- `IOS_PENDING_NOTIFICATION_LIMIT` is 64. The limit counts every pending request on the device, so
  pass a lower number when other features schedule too.
- The outcome reports list, cancel, permission, schedule, and limit failures by code and logical ID,
  never with the notification content.

The module is passed in, and the package imports only types from `expo-notifications`, so the
package itself loads in Node and in tests. The tests run the core conformance suite against a
mocked module.

## Boundaries

The package does not request permission, create Android channels, set a notification handler, or
choose copy. It does not support repeating triggers; plan a rolling horizon and replace it when the
app opens or the plan changes.
