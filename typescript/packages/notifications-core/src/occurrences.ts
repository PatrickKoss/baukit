import {
  addCivilDays,
  civilDateForInstant,
  compareCivilDates,
  INVALID_TIME_ZONE_CODE,
  NONEXISTENT_LOCAL_TIME_CODE,
  resolveZonedLocalTime,
  type FoldPolicy,
  type GapPolicy,
  type LocalTimeTransition,
  type ZonedLocalTimeResult,
} from '@baukit/localization-core';

import { NotificationPlanError } from './errors.js';
import { assertIdentity, compareByInstant, compareCodeUnits } from './validation.js';

export interface NotificationOccurrence {
  readonly logicalId: string;
  readonly civilDate: string;
  readonly civilTime: string;
  readonly contentDigest: string;
}

export type NotificationClock = () => number;

export interface NotificationOccurrenceInput<T extends NotificationOccurrence> {
  readonly occurrences: readonly T[];
  readonly timeZone: string;
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly clock: NotificationClock;
  readonly horizonDays: number;
}

export type ResolvedNotification<T extends NotificationOccurrence = NotificationOccurrence> = T & {
  readonly epochMilliseconds: number;
  readonly transition: LocalTimeTransition;
};

export type SkippedOccurrenceReason = 'past' | 'outside_horizon' | 'nonexistent_local_time';

export interface SkippedOccurrence {
  readonly logicalId: string;
  readonly reason: SkippedOccurrenceReason;
}

export interface NotificationOccurrenceResolution<T extends NotificationOccurrence> {
  readonly now: number;
  readonly firstCivilDate: string;
  readonly lastCivilDate: string;
  readonly desired: readonly ResolvedNotification<T>[];
  readonly skipped: readonly SkippedOccurrence[];
}

interface HorizonWindow {
  readonly now: number;
  readonly firstCivilDate: string;
  readonly lastCivilDate: string;
}

const TIME_ZONE_PROBE_DATE = '2000-01-01';
const TIME_ZONE_PROBE_TIME = '12:00';

export function resolveNotificationOccurrences<T extends NotificationOccurrence>(
  input: NotificationOccurrenceInput<T>,
): NotificationOccurrenceResolution<T> {
  const window = horizonWindow(input);
  const seen = new Set<string>();
  const desired: ResolvedNotification<T>[] = [];
  const skipped: SkippedOccurrence[] = [];

  for (const occurrence of input.occurrences) {
    assertIdentity(occurrence, seen);
    const result = resolveZonedLocalTime({
      civilDate: occurrence.civilDate,
      civilTime: occurrence.civilTime,
      timeZone: input.timeZone,
      gap: input.gap,
      fold: input.fold,
    });
    const reason = skipReason(occurrence, result, window);
    if (reason !== null) {
      skipped.push({ logicalId: occurrence.logicalId, reason });
    } else if (result.ok) {
      desired.push({
        ...occurrence,
        epochMilliseconds: result.epochMilliseconds,
        transition: result.transition,
      });
    }
  }

  return {
    ...window,
    desired: desired.sort(compareByInstant),
    skipped: skipped.sort((left, right) => compareCodeUnits(left.logicalId, right.logicalId)),
  };
}

function horizonWindow(input: NotificationOccurrenceInput<NotificationOccurrence>): HorizonWindow {
  assertTimeZoneAndPolicies(input);
  if (!Number.isSafeInteger(input.horizonDays) || input.horizonDays < 1) {
    throw new NotificationPlanError('invalid_horizon');
  }
  const now = input.clock();
  if (!Number.isFinite(now) || Number.isNaN(new Date(now).getTime())) {
    throw new NotificationPlanError('invalid_clock');
  }
  const firstCivilDate = civilDateForInstant(now, input.timeZone);
  try {
    return {
      now,
      firstCivilDate,
      lastCivilDate: addCivilDays(firstCivilDate, input.horizonDays - 1),
    };
  } catch {
    throw new NotificationPlanError('invalid_horizon');
  }
}

function assertTimeZoneAndPolicies(
  input: NotificationOccurrenceInput<NotificationOccurrence>,
): void {
  const probe = resolveZonedLocalTime({
    civilDate: TIME_ZONE_PROBE_DATE,
    civilTime: TIME_ZONE_PROBE_TIME,
    timeZone: input.timeZone,
    gap: input.gap,
    fold: input.fold,
  });
  if (!probe.ok && probe.code === INVALID_TIME_ZONE_CODE) {
    throw new NotificationPlanError('invalid_time_zone');
  }
}

function skipReason(
  occurrence: NotificationOccurrence,
  result: ZonedLocalTimeResult,
  window: HorizonWindow,
): SkippedOccurrenceReason | null {
  if (!result.ok && result.code !== NONEXISTENT_LOCAL_TIME_CODE) {
    throw new NotificationPlanError(result.code, occurrence.logicalId);
  }
  if (
    compareCivilDates(occurrence.civilDate, window.firstCivilDate) < 0 ||
    compareCivilDates(occurrence.civilDate, window.lastCivilDate) > 0
  ) {
    return 'outside_horizon';
  }
  if (!result.ok) {
    return 'nonexistent_local_time';
  }
  return result.epochMilliseconds <= window.now ? 'past' : null;
}
