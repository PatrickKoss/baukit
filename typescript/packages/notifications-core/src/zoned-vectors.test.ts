import { readFileSync } from 'node:fs';

import type { FoldPolicy, GapPolicy, LocalTimeTransition } from '@baukit/localization-core';
import { describe, expect, it } from 'vitest';

import { NotificationPlanError } from './errors.js';
import { resolveNotificationOccurrences } from './occurrences.js';

interface ZonedTimeOutcome {
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly instant?: string;
  readonly error?: string;
}

interface ZonedTimeCase {
  readonly name: string;
  readonly civilDate: string;
  readonly civilTime: string;
  readonly timeZone: string;
  readonly transition: LocalTimeTransition | null;
  readonly outcomes: readonly ZonedTimeOutcome[];
}

interface ZonedTimeFixture {
  readonly cases: readonly ZonedTimeCase[];
}

const fixtureUrl = new URL('../../../../fixtures/zoned-time/vectors-v1.json', import.meta.url);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixture = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as ZonedTimeFixture;

const MILLISECONDS_PER_DAY = 86_400_000;
const DAYS_BEFORE_OCCURRENCE = 2;
const HORIZON_DAYS = 5;
const FALLBACK_NOW = Date.parse('2026-01-01T12:00:00Z');
const LOGICAL_ID = 'occurrence';
const NONEXISTENT = 'nonexistent_local_time';

function nowBefore(civilDate: string): number {
  const noon = Date.parse(`${civilDate}T12:00:00Z`);
  return Number.isNaN(noon) ? FALLBACK_NOW : noon - DAYS_BEFORE_OCCURRENCE * MILLISECONDS_PER_DAY;
}

function resolve(entry: ZonedTimeCase, outcome: ZonedTimeOutcome) {
  return resolveNotificationOccurrences({
    occurrences: [
      {
        logicalId: LOGICAL_ID,
        civilDate: entry.civilDate,
        civilTime: entry.civilTime,
        contentDigest: 'v1',
      },
    ],
    timeZone: entry.timeZone,
    gap: outcome.gap,
    fold: outcome.fold,
    clock: () => nowBefore(entry.civilDate),
    horizonDays: HORIZON_DAYS,
  });
}

function thrown(action: () => unknown): unknown {
  try {
    action();
  } catch (error) {
    return error;
  }
  return undefined;
}

const outcomeCases = fixture.cases.flatMap((entry) =>
  entry.outcomes.map(
    (outcome) => [`${entry.name} gap=${outcome.gap} fold=${outcome.fold}`, entry, outcome] as const,
  ),
);
const scheduled = outcomeCases.filter(([, , outcome]) => outcome.error === undefined);
const skipped = outcomeCases.filter(([, , outcome]) => outcome.error === NONEXISTENT);
const rejected = outcomeCases.filter(
  ([, , outcome]) => outcome.error !== undefined && outcome.error !== NONEXISTENT,
);

describe('zoned local time vectors through notification planning', () => {
  it.each(scheduled)('%s schedules the vector instant', (_label, entry, outcome) => {
    const resolution = resolve(entry, outcome);
    expect(resolution.skipped).toEqual([]);
    expect(resolution.desired).toMatchObject([
      {
        logicalId: LOGICAL_ID,
        epochMilliseconds: Date.parse(outcome.instant ?? ''),
        transition: entry.transition,
      },
    ]);
  });

  it.each(skipped)('%s skips the missing local time', (_label, entry, outcome) => {
    const resolution = resolve(entry, outcome);
    expect(resolution.desired).toEqual([]);
    expect(resolution.skipped).toEqual([{ logicalId: LOGICAL_ID, reason: NONEXISTENT }]);
  });

  it.each(rejected)('%s rejects the input', (_label, entry, outcome) => {
    const error = thrown(() => resolve(entry, outcome));
    expect(error).toBeInstanceOf(NotificationPlanError);
    expect(error).toMatchObject({ code: outcome.error });
  });
});
