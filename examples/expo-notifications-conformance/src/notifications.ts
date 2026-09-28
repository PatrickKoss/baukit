import {
  encodeOwnedNotificationMarker,
  isOwnedBy,
  OWNED_NOTIFICATION_DATA_KEY,
  ownedNotificationIdentifier,
  type NotificationOwner,
  type OwnedNotification,
  type PlannedNotification,
} from "@baukit/notifications-core";
import {
  createExpoNotificationPlatform,
  createExpoOwnedNotificationScheduler,
  type ExpoNotificationContent,
} from "@baukit/notifications-expo";
import * as Notifications from "expo-notifications";

import { assertEqual } from "./check";

type Spec = readonly [logicalId: string, hour: number, contentDigest?: string];

const MILLISECONDS_PER_HOUR = 3_600_000;
const HOURS_AHEAD = 24;
const PERMISSION_REQUEST_TIMEOUT_MS = 15_000;
const OWNER: NotificationOwner = { namespace: "reminders" };
const PREFIX_SIBLING: NotificationOwner = { namespace: "reminders-extra" };
const FOREIGN_IDENTIFIER = "foreign-request";

const platform = createExpoNotificationPlatform(Notifications);
const scheduler = createExpoOwnedNotificationScheduler(Notifications);

export interface GrantedResult {
  readonly permission: string;
  readonly trigger: unknown;
}

export interface DeniedResult {
  readonly status: string;
  readonly canAskAgain: boolean;
  readonly mapped: string;
}

export async function isPermissionGranted(): Promise<boolean> {
  return (await Notifications.getPermissionsAsync()).granted;
}

export async function runGranted(): Promise<GrantedResult> {
  const base = baseInstant();
  await Notifications.cancelAllScheduledNotificationsAsync();
  const permission = await platform.permission();
  assertEqual("mapped permission", permission, "granted");

  await scheduleForeign(base);
  const sibling = await scheduler.replaceOwned(
    PREFIX_SIBLING,
    build(base, [["s1", 1]]),
  );
  assertEqual("sibling owner", sibling.scheduled, ["s1"]);

  const first = await scheduler.replaceOwned(
    OWNER,
    build(base, [
      ["b", 3],
      ["a", 2],
      ["c", 4],
    ]),
  );
  assertEqual(
    "first replacement",
    first,
    outcome({ scheduled: ["a", "b", "c"] }),
  );
  assertEqual(
    "owned after first",
    await owned(OWNER),
    planned(base, [
      ["a", 2],
      ["b", 3],
      ["c", 4],
    ]),
  );
  const request = await requestOf(
    ownedNotificationIdentifier(OWNER.namespace, "a"),
  );
  const trigger: unknown = request?.trigger ?? null;
  assertEqual("date trigger of a", pick(trigger, ["type", "value"]), {
    type: "date",
    value: base + 2 * MILLISECONDS_PER_HOUR,
  });
  assertEqual(
    "content of a",
    { title: request?.content.title, route: request?.content.data?.route },
    { title: "Reminder a", route: "/today" },
  );

  const next: Spec[] = [
    ["a", 2],
    ["b", 3, "v2"],
    ["d", 5],
  ];
  const second = await scheduler.replaceOwned(OWNER, build(base, next));
  assertEqual(
    "second replacement",
    second,
    outcome({ kept: ["a"], cancelled: ["b", "c"], scheduled: ["b", "d"] }),
  );
  assertEqual("owned after second", await owned(OWNER), planned(base, next));
  assertEqual(
    "sibling after second",
    await owned(PREFIX_SIBLING),
    planned(base, [["s1", 1]]),
  );
  assertEqual(
    "foreign after second",
    await hasIdentifier(FOREIGN_IDENTIFIER),
    true,
  );

  const rerun = await scheduler.replaceOwned(OWNER, build(base, next));
  assertEqual("unchanged rerun", rerun, outcome({ kept: ["a", "b", "d"] }));

  const cleared = await scheduler.replaceOwned(OWNER, []);
  assertEqual(
    "empty replacement",
    cleared,
    outcome({ cancelled: ["a", "b", "d"] }),
  );
  assertEqual(
    "identifiers after clear",
    await identifiers(),
    [
      FOREIGN_IDENTIFIER,
      ownedNotificationIdentifier(PREFIX_SIBLING.namespace, "s1"),
    ].sort(),
  );

  await Notifications.cancelAllScheduledNotificationsAsync();
  return { permission, trigger };
}

export async function runDenied(): Promise<DeniedResult> {
  const base = baseInstant();
  await Notifications.cancelAllScheduledNotificationsAsync();
  const requested = await withTimeout(
    Notifications.requestPermissionsAsync(),
    PERMISSION_REQUEST_TIMEOUT_MS,
    "requestPermissionsAsync showed a prompt instead of answering",
  );
  assertEqual("requested permission granted", requested.granted, false);
  const current = await Notifications.getPermissionsAsync();
  const mapped = await platform.permission();
  assertEqual("mapped permission", mapped, "denied");

  await scheduleForeign(base);
  await scheduleLeftoverOwned(base, "stale", 6);
  const replacement = await scheduler.replaceOwned(
    OWNER,
    build(base, [
      ["a", 2],
      ["b", 3],
    ]),
  );
  assertEqual(
    "denied replacement",
    replacement,
    outcome({
      cancelled: ["stale"],
      failures: [
        { code: "permission_denied", logicalId: "a" },
        { code: "permission_denied", logicalId: "b" },
      ],
    }),
  );
  assertEqual("identifiers after denied replacement", await identifiers(), [
    FOREIGN_IDENTIFIER,
  ]);

  await Notifications.cancelAllScheduledNotificationsAsync();
  return { status: current.status, canAskAgain: current.canAskAgain, mapped };
}

function baseInstant(): number {
  return (
    (Math.floor(Date.now() / MILLISECONDS_PER_HOUR) + HOURS_AHEAD) *
    MILLISECONDS_PER_HOUR
  );
}

function entryOf(
  base: number,
  [logicalId, hour, contentDigest = "v1"]: Spec,
): PlannedNotification {
  return {
    logicalId,
    epochMilliseconds: base + hour * MILLISECONDS_PER_HOUR,
    contentDigest,
  };
}

function byInstant(
  left: PlannedNotification,
  right: PlannedNotification,
): number {
  return left.epochMilliseconds - right.epochMilliseconds;
}

function planned(base: number, specs: readonly Spec[]): PlannedNotification[] {
  return specs.map((spec) => entryOf(base, spec)).sort(byInstant);
}

function build(
  base: number,
  specs: readonly Spec[],
): OwnedNotification<ExpoNotificationContent>[] {
  return specs.map((spec) => {
    const entry = entryOf(base, spec);
    return {
      ...entry,
      content: {
        content: {
          title: `Reminder ${entry.logicalId}`,
          data: { route: "/today" },
        },
      },
    };
  });
}

function outcome(parts: {
  readonly kept?: readonly string[];
  readonly cancelled?: readonly string[];
  readonly scheduled?: readonly string[];
  readonly failures?: readonly {
    readonly code: string;
    readonly logicalId: string;
  }[];
}) {
  const failures = parts.failures ?? [];
  return {
    status: failures.length === 0 ? "complete" : "incomplete",
    kept: parts.kept ?? [],
    cancelled: parts.cancelled ?? [],
    scheduled: parts.scheduled ?? [],
    failures,
  };
}

async function owned(owner: NotificationOwner): Promise<PlannedNotification[]> {
  const pending = await platform.list();
  return pending
    .flatMap(({ identifier, marker }) =>
      isOwnedBy(owner.namespace, identifier, marker) ? [marker] : [],
    )
    .map(({ logicalId, epochMilliseconds, contentDigest }) => ({
      logicalId,
      epochMilliseconds,
      contentDigest,
    }))
    .sort(byInstant);
}

async function identifiers(): Promise<string[]> {
  const pending = await Notifications.getAllScheduledNotificationsAsync();
  return pending.map(({ identifier }) => identifier).sort();
}

async function hasIdentifier(identifier: string): Promise<boolean> {
  return (await identifiers()).includes(identifier);
}

async function requestOf(
  identifier: string,
): Promise<Notifications.NotificationRequest | undefined> {
  const pending = await Notifications.getAllScheduledNotificationsAsync();
  return pending.find((request) => request.identifier === identifier);
}

function pick(
  value: unknown,
  keys: readonly string[],
): Record<string, unknown> {
  if (value === null || typeof value !== "object") return {};
  const record = value as Record<string, unknown>;
  return Object.fromEntries(keys.map((key) => [key, record[key]]));
}

async function scheduleForeign(base: number): Promise<void> {
  await Notifications.scheduleNotificationAsync({
    identifier: FOREIGN_IDENTIFIER,
    content: { title: "Foreign reminder", data: { kind: "foreign" } },
    trigger: {
      type: Notifications.SchedulableTriggerInputTypes.DATE,
      date: base,
    },
  });
}

async function scheduleLeftoverOwned(
  base: number,
  logicalId: string,
  hour: number,
): Promise<void> {
  const epochMilliseconds = base + hour * MILLISECONDS_PER_HOUR;
  const marker = {
    namespace: OWNER.namespace,
    logicalId,
    epochMilliseconds,
    contentDigest: "v1",
  };
  await Notifications.scheduleNotificationAsync({
    identifier: ownedNotificationIdentifier(OWNER.namespace, logicalId),
    content: {
      title: `Reminder ${logicalId}`,
      data: {
        [OWNED_NOTIFICATION_DATA_KEY]: encodeOwnedNotificationMarker(marker),
      },
    },
    trigger: {
      type: Notifications.SchedulableTriggerInputTypes.DATE,
      date: epochMilliseconds,
    },
  });
}

function withTimeout<T>(
  promise: Promise<T>,
  milliseconds: number,
  message: string,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(message)), milliseconds);
    promise.then(
      (value) => {
        clearTimeout(timer);
        resolve(value);
      },
      (error: unknown) => {
        clearTimeout(timer);
        reject(error instanceof Error ? error : new Error(String(error)));
      },
    );
  });
}
