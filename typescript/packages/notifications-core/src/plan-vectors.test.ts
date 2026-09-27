import { readFileSync } from 'node:fs';

import type { FoldPolicy, GapPolicy, LocalTimeTransition } from '@baukit/localization-core';
import { describe, expect, it } from 'vitest';

import { NotificationPlanError } from './errors.js';
import {
  resolveNotificationOccurrences,
  type NotificationOccurrence,
  type SkippedOccurrence,
} from './occurrences.js';
import { planNotificationReplacement } from './replacement-plan.js';
import type { PlannedNotification } from './validation.js';

interface CurrentVector {
  readonly logicalId: string;
  readonly instant: string;
  readonly contentDigest: string;
}

interface PlanVectorInput {
  readonly timeZone: string;
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly now: string;
  readonly horizonDays: number;
  readonly replaceAll?: boolean;
  readonly occurrences: readonly NotificationOccurrence[];
  readonly current: readonly CurrentVector[];
}

interface PlanVectorResult {
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

interface PlanVectorError {
  readonly error: { readonly code: string; readonly logicalId?: string };
}

interface PlanVectorCase {
  readonly name: string;
  readonly input: PlanVectorInput;
  readonly expected: PlanVectorResult | PlanVectorError;
}

interface PlanVectorFixture {
  readonly version: number;
  readonly cases: readonly PlanVectorCase[];
}

const fixtureUrl = new URL(
  '../../../../fixtures/notifications/plan-vectors-v1.json',
  import.meta.url,
);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixture = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as PlanVectorFixture;

function toCurrent(entries: readonly CurrentVector[]): PlannedNotification[] {
  return entries.map(({ logicalId, instant, contentDigest }) => ({
    logicalId,
    epochMilliseconds: Date.parse(instant),
    contentDigest,
  }));
}

function run(input: PlanVectorInput, current: readonly PlannedNotification[]) {
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
  input: PlanVectorInput,
  current: readonly PlannedNotification[],
): PlanVectorResult {
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

function thrown(action: () => unknown): unknown {
  try {
    action();
  } catch (error) {
    return error;
  }
  return undefined;
}

const successes = fixture.cases.filter(
  (entry): entry is PlanVectorCase & { expected: PlanVectorResult } => !('error' in entry.expected),
);
const failures = fixture.cases.filter(
  (entry): entry is PlanVectorCase & { expected: PlanVectorError } => 'error' in entry.expected,
);

describe('notification plan vectors', () => {
  it('reads version 1 of the shared vectors', () => {
    expect(fixture.version).toBe(1);
    expect(successes.length).toBeGreaterThan(0);
    expect(failures.length).toBeGreaterThan(0);
  });

  it.each(successes.map((entry) => [entry.name, entry] as const))('%s', (_name, entry) => {
    expect(summary(entry.input, toCurrent(entry.input.current))).toEqual(entry.expected);
  });

  it.each(failures.map((entry) => [entry.name, entry] as const))('%s', (_name, entry) => {
    const error = thrown(() => run(entry.input, toCurrent(entry.input.current)));
    expect(error).toBeInstanceOf(NotificationPlanError);
    expect(error).toMatchObject({
      code: entry.expected.error.code,
      logicalId: entry.expected.error.logicalId,
    });
  });

  it.each(successes.map((entry) => [entry.name, entry] as const))(
    '%s converges after the plan is applied',
    (_name, entry) => {
      const { plan } = run(entry.input, toCurrent(entry.input.current));
      const applied = [...plan.keep, ...plan.schedule].map(
        ({ logicalId, epochMilliseconds, contentDigest }) => ({
          logicalId,
          epochMilliseconds,
          contentDigest,
        }),
      );
      const settled = summary({ ...entry.input, replaceAll: false }, applied.reverse());
      expect(settled.schedule).toEqual([]);
      expect(settled.cancel).toEqual([]);
      expect(settled.keep).toHaveLength(applied.length);
    },
  );

  it.each(successes.map((entry) => [entry.name, entry] as const))(
    '%s does not depend on input order',
    (_name, entry) => {
      const reversed = { ...entry.input, occurrences: [...entry.input.occurrences].reverse() };
      const current = toCurrent(entry.input.current).reverse();
      expect(summary(reversed, current)).toEqual(entry.expected);
    },
  );
});
