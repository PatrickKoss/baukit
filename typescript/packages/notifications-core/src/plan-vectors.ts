import type { FoldPolicy, GapPolicy, LocalTimeTransition } from '@baukit/localization-core';

import { NotificationPlanError } from './errors.js';
import {
  resolveNotificationOccurrences,
  type NotificationOccurrence,
  type SkippedOccurrence,
} from './occurrences.js';
import { planNotificationReplacement } from './replacement-plan.js';
import type { PlannedNotification } from './validation.js';

export interface NotificationPlanVectorCurrent {
  readonly logicalId: string;
  readonly instant: string;
  readonly contentDigest: string;
}

export interface NotificationPlanVectorInput {
  readonly timeZone: string;
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly now: string;
  readonly horizonDays: number;
  readonly replaceAll?: boolean;
  readonly occurrences: readonly NotificationOccurrence[];
  readonly current: readonly NotificationPlanVectorCurrent[];
}

export interface NotificationPlanVectorResult {
  readonly firstCivilDate: string;
  readonly lastCivilDate: string;
  readonly schedule: readonly {
    readonly logicalId: string;
    readonly instant: string;
    readonly transition: LocalTimeTransition;
  }[];
  readonly keep: readonly string[];
  readonly cancel: readonly string[];
  readonly skipped: readonly SkippedOccurrence[];
}

export interface NotificationPlanVectorError {
  readonly error: { readonly code: string; readonly logicalId?: string };
}

export interface NotificationPlanVectorCase {
  readonly name: string;
  readonly input: NotificationPlanVectorInput;
  readonly expected: NotificationPlanVectorResult | NotificationPlanVectorError;
}

export interface NotificationPlanVectorFixture {
  readonly version: number;
  readonly cases: readonly NotificationPlanVectorCase[];
}

/** One vector: `actual()` must deep-equal `expected` (`undefined` fields count as absent). */
export interface NotificationPlanVectorCheck {
  readonly label: string;
  readonly expected: unknown;
  readonly actual: () => unknown;
}

interface Convergence {
  readonly schedule: readonly unknown[];
  readonly cancel: readonly unknown[];
  readonly notKept: number;
}

/**
 * Expands `fixtures/notifications/plan-vectors-v1.json` into checks. Each success case yields
 * three: the expected plan, convergence once the plan is applied, and independence from input
 * order. Each error case yields one check of the thrown `NotificationPlanError` code and ID.
 */
export function notificationPlanVectorChecks(
  fixture: NotificationPlanVectorFixture,
): NotificationPlanVectorCheck[] {
  return fixture.cases.flatMap((entry) =>
    isErrorCase(entry) ? [errorCheck(entry)] : successChecks(entry),
  );
}

function isErrorCase(
  entry: NotificationPlanVectorCase,
): entry is NotificationPlanVectorCase & { expected: NotificationPlanVectorError } {
  return 'error' in entry.expected;
}

function successChecks(entry: NotificationPlanVectorCase): NotificationPlanVectorCheck[] {
  const { input } = entry;
  return [
    {
      label: entry.name,
      expected: entry.expected,
      actual: () => summary(input, toCurrent(input.current)),
    },
    {
      label: `${entry.name} converges after the plan is applied`,
      expected: { schedule: [], cancel: [], notKept: 0 },
      actual: () => convergence(input),
    },
    {
      label: `${entry.name} does not depend on input order`,
      expected: entry.expected,
      actual: () =>
        summary(
          { ...input, occurrences: [...input.occurrences].reverse() },
          toCurrent(input.current).reverse(),
        ),
    },
  ];
}

function errorCheck(
  entry: NotificationPlanVectorCase & { expected: NotificationPlanVectorError },
): NotificationPlanVectorCheck {
  const { code, logicalId } = entry.expected.error;
  return {
    label: entry.name,
    expected: { error: { code, logicalId } },
    actual: () => thrownPlanError(() => run(entry.input, toCurrent(entry.input.current))),
  };
}

function toCurrent(entries: readonly NotificationPlanVectorCurrent[]): PlannedNotification[] {
  return entries.map(({ logicalId, instant, contentDigest }) => ({
    logicalId,
    epochMilliseconds: Date.parse(instant),
    contentDigest,
  }));
}

function run(input: NotificationPlanVectorInput, current: readonly PlannedNotification[]) {
  const resolution = resolveNotificationOccurrences({
    occurrences: input.occurrences,
    timeZone: input.timeZone,
    gap: input.gap,
    fold: input.fold,
    clock: () => Date.parse(input.now),
    horizonDays: input.horizonDays,
  });
  const plan = planNotificationReplacement(current, resolution.desired, {
    replaceAll: input.replaceAll ?? false,
  });
  return { resolution, plan };
}

function summary(
  input: NotificationPlanVectorInput,
  current: readonly PlannedNotification[],
): NotificationPlanVectorResult {
  const { resolution, plan } = run(input, current);
  return {
    firstCivilDate: resolution.firstCivilDate,
    lastCivilDate: resolution.lastCivilDate,
    schedule: plan.schedule.map((entry) => ({
      logicalId: entry.logicalId,
      instant: new Date(entry.epochMilliseconds).toISOString().replace('.000Z', 'Z'),
      transition: entry.transition,
    })),
    keep: plan.keep.map((entry) => entry.logicalId),
    cancel: plan.cancel.map((entry) => entry.logicalId),
    skipped: resolution.skipped,
  };
}

function appliedPlan(input: NotificationPlanVectorInput): PlannedNotification[] {
  const { plan } = run(input, toCurrent(input.current));
  return [...plan.keep, ...plan.schedule].map(
    ({ logicalId, epochMilliseconds, contentDigest }) => ({
      logicalId,
      epochMilliseconds,
      contentDigest,
    }),
  );
}

function convergence(input: NotificationPlanVectorInput): Convergence {
  const applied = appliedPlan(input);
  const settled = summary({ ...input, replaceAll: false }, [...applied].reverse());
  return {
    schedule: settled.schedule,
    cancel: settled.cancel,
    notKept: applied.length - settled.keep.length,
  };
}

function thrownPlanError(action: () => unknown): unknown {
  try {
    action();
  } catch (error) {
    if (error instanceof NotificationPlanError) {
      return { error: { code: error.code, logicalId: error.logicalId } };
    }
    return { unexpected: String(error) };
  }
  return { unexpected: 'no error thrown' };
}
