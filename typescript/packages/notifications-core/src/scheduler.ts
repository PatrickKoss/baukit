import { NotificationPlanError } from './errors.js';
import {
  isOwnedBy,
  ownedNotificationIdentifier,
  type OwnedNotificationMarker,
} from './ownership.js';
import {
  planNotificationReplacement,
  type NotificationReplacementOptions,
} from './replacement-plan.js';
import {
  assertNamespace,
  assertPlannedNotifications,
  type PlannedNotification,
} from './validation.js';

export type NotificationPermission = 'granted' | 'denied' | 'undetermined';

export interface PendingNotification {
  readonly identifier: string;
  readonly marker: OwnedNotificationMarker | null;
}

export interface OwnedNotificationScheduleRequest<TContent> {
  readonly identifier: string;
  readonly marker: OwnedNotificationMarker;
  readonly content: TContent;
}

export interface NotificationPlatform<TContent> {
  list(): Promise<readonly PendingNotification[]>;
  cancel(identifier: string): Promise<void>;
  permission(): Promise<NotificationPermission>;
  schedule(request: OwnedNotificationScheduleRequest<TContent>): Promise<void>;
}

export interface NotificationOwner {
  readonly namespace: string;
}

export interface OwnedNotification<TContent> extends PlannedNotification {
  readonly content: TContent;
}

export type OwnedNotificationFailureCode =
  | 'list_failed'
  | 'cancel_failed'
  | 'permission_denied'
  | 'permission_failed'
  | 'schedule_failed'
  | 'schedule_limit';

export interface OwnedNotificationFailure {
  readonly code: OwnedNotificationFailureCode;
  readonly logicalId?: string;
}

export type OwnedNotificationReplacementStatus = 'complete' | 'incomplete' | 'superseded';

export interface OwnedNotificationReplacementOutcome {
  readonly status: OwnedNotificationReplacementStatus;
  readonly kept: readonly string[];
  readonly cancelled: readonly string[];
  readonly scheduled: readonly string[];
  readonly failures: readonly OwnedNotificationFailure[];
}

export interface OwnedNotificationScheduler<TContent> {
  replaceOwned(
    owner: NotificationOwner,
    desired: readonly OwnedNotification<TContent>[],
    options?: NotificationReplacementOptions,
  ): Promise<OwnedNotificationReplacementOutcome>;
}

export interface OwnedNotificationSchedulerOptions {
  readonly pendingLimit?: number;
}

type Replacement = () => Promise<OwnedNotificationReplacementOutcome>;

interface QueuedReplacement {
  readonly run: Replacement;
  readonly resolve: (outcome: OwnedNotificationReplacementOutcome) => void;
  readonly reject: (error: unknown) => void;
}

interface OwnerQueue {
  waiting: QueuedReplacement | null;
}

interface ScheduleResult {
  readonly scheduled: readonly string[];
  readonly failures: readonly OwnedNotificationFailure[];
}

const SUPERSEDED: OwnedNotificationReplacementOutcome = Object.freeze({
  status: 'superseded',
  kept: [],
  cancelled: [],
  scheduled: [],
  failures: [],
});

export function createOwnedNotificationScheduler<TContent>(
  platform: NotificationPlatform<TContent>,
  options: OwnedNotificationSchedulerOptions = {},
): OwnedNotificationScheduler<TContent> {
  const pendingLimit = options.pendingLimit ?? Number.POSITIVE_INFINITY;
  if (pendingLimit !== Number.POSITIVE_INFINITY && !isPositiveInteger(pendingLimit)) {
    throw new NotificationPlanError('invalid_pending_limit');
  }
  const queues = new Map<string, OwnerQueue>();

  return {
    async replaceOwned(owner, desired, replaceOptions = {}) {
      assertNamespace(owner.namespace);
      assertPlannedNotifications(desired);
      const namespace = owner.namespace;
      const snapshot = [...desired];
      return enqueue(queues, namespace, () =>
        replaceNow(platform, namespace, snapshot, replaceOptions, pendingLimit),
      );
    },
  };
}

function enqueue(
  queues: Map<string, OwnerQueue>,
  namespace: string,
  run: Replacement,
): Promise<OwnedNotificationReplacementOutcome> {
  return new Promise((resolve, reject) => {
    const replacement = { run, resolve, reject };
    const queue = queues.get(namespace);
    if (queue === undefined) {
      const created: OwnerQueue = { waiting: null };
      queues.set(namespace, created);
      start(queues, namespace, created, replacement);
      return;
    }
    queue.waiting?.resolve(SUPERSEDED);
    queue.waiting = replacement;
  });
}

function start(
  queues: Map<string, OwnerQueue>,
  namespace: string,
  queue: OwnerQueue,
  replacement: QueuedReplacement,
): void {
  void replacement
    .run()
    .then(replacement.resolve, replacement.reject)
    .finally(() => {
      const next = queue.waiting;
      queue.waiting = null;
      if (next === null) {
        queues.delete(namespace);
        return;
      }
      start(queues, namespace, queue, next);
    });
}

async function replaceNow<TContent>(
  platform: NotificationPlatform<TContent>,
  namespace: string,
  desired: readonly OwnedNotification<TContent>[],
  options: NotificationReplacementOptions,
  pendingLimit: number,
): Promise<OwnedNotificationReplacementOutcome> {
  let pending: readonly PendingNotification[];
  try {
    pending = await platform.list();
  } catch {
    return outcome([], [], [], [{ code: 'list_failed' }]);
  }

  const current = pending.flatMap(({ identifier, marker }) =>
    isOwnedBy(namespace, identifier, marker) ? [marker] : [],
  );
  const plan = planNotificationReplacement(current, desired, options);

  const cancelFailures: OwnedNotificationFailure[] = [];
  const cancelled: string[] = [];
  for (const entry of plan.cancel) {
    try {
      await platform.cancel(ownedNotificationIdentifier(namespace, entry.logicalId));
      cancelled.push(entry.logicalId);
    } catch {
      cancelFailures.push({ code: 'cancel_failed', logicalId: entry.logicalId });
    }
  }

  const blocked = new Set(cancelFailures.map((failure) => failure.logicalId));
  const toSchedule = plan.schedule.filter((entry) => !blocked.has(entry.logicalId));
  const capacity = pendingLimit - (pending.length - cancelled.length);
  const scheduling = await scheduleAll(platform, namespace, toSchedule, capacity);

  return outcome(
    plan.keep.map((entry) => entry.logicalId),
    cancelled,
    scheduling.scheduled,
    [...cancelFailures, ...scheduling.failures],
  );
}

async function scheduleAll<TContent>(
  platform: NotificationPlatform<TContent>,
  namespace: string,
  entries: readonly OwnedNotification<TContent>[],
  capacity: number,
): Promise<ScheduleResult> {
  if (entries.length === 0) {
    return { scheduled: [], failures: [] };
  }
  const blockedCode = await permissionFailure(platform);
  if (blockedCode !== null) {
    return {
      scheduled: [],
      failures: entries.map((entry) => ({ code: blockedCode, logicalId: entry.logicalId })),
    };
  }

  const scheduled: string[] = [];
  const failures: OwnedNotificationFailure[] = [];
  for (const entry of entries) {
    if (scheduled.length >= capacity) {
      failures.push({ code: 'schedule_limit', logicalId: entry.logicalId });
      continue;
    }
    try {
      await platform.schedule(scheduleRequest(namespace, entry));
      scheduled.push(entry.logicalId);
    } catch {
      failures.push({ code: 'schedule_failed', logicalId: entry.logicalId });
    }
  }
  return { scheduled, failures };
}

async function permissionFailure<TContent>(
  platform: NotificationPlatform<TContent>,
): Promise<OwnedNotificationFailureCode | null> {
  try {
    return (await platform.permission()) === 'granted' ? null : 'permission_denied';
  } catch {
    return 'permission_failed';
  }
}

function scheduleRequest<TContent>(
  namespace: string,
  entry: OwnedNotification<TContent>,
): OwnedNotificationScheduleRequest<TContent> {
  return {
    identifier: ownedNotificationIdentifier(namespace, entry.logicalId),
    marker: {
      namespace,
      logicalId: entry.logicalId,
      epochMilliseconds: entry.epochMilliseconds,
      contentDigest: entry.contentDigest,
    },
    content: entry.content,
  };
}

function outcome(
  kept: readonly string[],
  cancelled: readonly string[],
  scheduled: readonly string[],
  failures: readonly OwnedNotificationFailure[],
): OwnedNotificationReplacementOutcome {
  return {
    status: failures.length === 0 ? 'complete' : 'incomplete',
    kept,
    cancelled,
    scheduled,
    failures,
  };
}

function isPositiveInteger(value: number): boolean {
  return Number.isSafeInteger(value) && value > 0;
}
