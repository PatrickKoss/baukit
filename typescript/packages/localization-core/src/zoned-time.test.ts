import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  INVALID_TIME_ZONE_CODE,
  NONEXISTENT_LOCAL_TIME_CODE,
  resolveZonedLocalTime,
  type FoldPolicy,
  type GapPolicy,
  type LocalTimeTransition,
  type ZonedLocalTime,
  type ZonedLocalTimeCode,
} from './zoned-time.js';

interface ZonedTimeOutcome {
  readonly gap: GapPolicy;
  readonly fold: FoldPolicy;
  readonly instant?: string;
  readonly error?: ZonedLocalTimeCode;
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
  readonly version: number;
  readonly gapPolicies: readonly GapPolicy[];
  readonly foldPolicies: readonly FoldPolicy[];
  readonly cases: readonly ZonedTimeCase[];
}

const fixtureUrl = new URL('../../../../fixtures/zoned-time/vectors-v1.json', import.meta.url);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixture = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as ZonedTimeFixture;

const outcomeCases = fixture.cases.flatMap((entry) =>
  entry.outcomes.map((outcome) => ({
    ...outcome,
    label: `${entry.name} gap=${outcome.gap} fold=${outcome.fold}`,
    entry,
  })),
);

function expectedResult(entry: ZonedTimeCase, outcome: ZonedTimeOutcome): unknown {
  if (outcome.error !== undefined) {
    return { ok: false, code: outcome.error };
  }
  return {
    ok: true,
    epochMilliseconds: Date.parse(outcome.instant ?? ''),
    transition: entry.transition,
  };
}

const berlinGap: ZonedLocalTime = {
  civilDate: '2026-03-29',
  civilTime: '02:30',
  timeZone: 'Europe/Berlin',
  gap: 'reject',
  fold: 'earlier',
};

describe('zoned local time vectors', () => {
  it('reads version 1 of the shared vectors', () => {
    expect(fixture.version).toBe(1);
  });

  it('covers every gap and fold policy pair for every case', () => {
    const pairs = fixture.gapPolicies.length * fixture.foldPolicies.length;
    for (const entry of fixture.cases) {
      expect(entry.outcomes).toHaveLength(pairs);
    }
  });

  it.each(outcomeCases)('$label', ({ entry, gap, fold, ...outcome }) => {
    const { civilDate, civilTime, timeZone } = entry;
    expect(resolveZonedLocalTime({ civilDate, civilTime, timeZone, gap, fold })).toEqual(
      expectedResult(entry, { gap, fold, ...outcome }),
    );
  });
});

describe('resolveZonedLocalTime', () => {
  it('rejects a gap only under the reject policy', () => {
    expect(resolveZonedLocalTime(berlinGap)).toEqual({
      ok: false,
      code: NONEXISTENT_LOCAL_TIME_CODE,
    });
    expect(resolveZonedLocalTime({ ...berlinGap, gap: 'shiftForward' })).toEqual({
      ok: true,
      epochMilliseconds: Date.parse('2026-03-29T01:30:00Z'),
      transition: 'gap',
    });
  });

  it('accepts a zone name in any case the runtime canonicalizes', () => {
    expect(
      resolveZonedLocalTime({ ...berlinGap, civilDate: '2026-06-15', timeZone: 'europe/berlin' }),
    ).toEqual({
      ok: true,
      epochMilliseconds: Date.parse('2026-06-15T00:30:00Z'),
      transition: 'none',
    });
  });

  it('rejects a zone with surrounding whitespace', () => {
    expect(resolveZonedLocalTime({ ...berlinGap, timeZone: ' Europe/Berlin' })).toEqual({
      ok: false,
      code: INVALID_TIME_ZONE_CODE,
    });
  });

  it.each([undefined, 'compatible', 'earlier'])('throws for gap policy %p', (gap) => {
    expect(() => resolveZonedLocalTime({ ...berlinGap, gap: gap as GapPolicy })).toThrow(
      RangeError,
    );
  });

  it.each([undefined, 'compatible', 'reject'])('throws for fold policy %p', (fold) => {
    expect(() => resolveZonedLocalTime({ ...berlinGap, fold: fold as FoldPolicy })).toThrow(
      RangeError,
    );
  });

  it('checks the policies before the inputs', () => {
    expect(() =>
      resolveZonedLocalTime({ ...berlinGap, civilDate: 'nope', gap: undefined as never }),
    ).toThrow(RangeError);
  });
});
