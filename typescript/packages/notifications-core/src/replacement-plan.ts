import {
  assertPlannedNotifications,
  compareByInstant,
  type PlannedNotification,
} from './validation.js';

export interface NotificationReplacementOptions {
  readonly replaceAll?: boolean;
}

export interface NotificationReplacementPlan<
  TCurrent extends PlannedNotification,
  TDesired extends PlannedNotification,
> {
  readonly keep: readonly TCurrent[];
  readonly cancel: readonly TCurrent[];
  readonly schedule: readonly TDesired[];
}

export function planNotificationReplacement<
  TCurrent extends PlannedNotification,
  TDesired extends PlannedNotification,
>(
  current: readonly TCurrent[],
  desired: readonly TDesired[],
  options: NotificationReplacementOptions = {},
): NotificationReplacementPlan<TCurrent, TDesired> {
  assertPlannedNotifications(current);
  assertPlannedNotifications(desired);

  const desiredById = new Map(desired.map((entry) => [entry.logicalId, entry]));
  const keep: TCurrent[] = [];
  const cancel: TCurrent[] = [];
  const kept = new Set<string>();

  for (const entry of current) {
    const wanted = desiredById.get(entry.logicalId);
    if (options.replaceAll !== true && wanted !== undefined && samePlacement(entry, wanted)) {
      keep.push(entry);
      kept.add(entry.logicalId);
    } else {
      cancel.push(entry);
    }
  }

  return {
    keep: keep.sort(compareByInstant),
    cancel: cancel.sort(compareByInstant),
    schedule: desired.filter((entry) => !kept.has(entry.logicalId)).sort(compareByInstant),
  };
}

function samePlacement(left: PlannedNotification, right: PlannedNotification): boolean {
  return (
    left.epochMilliseconds === right.epochMilliseconds && left.contentDigest === right.contentDigest
  );
}
