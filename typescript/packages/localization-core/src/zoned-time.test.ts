import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  INVALID_TIME_ZONE_CODE,
  NONEXISTENT_LOCAL_TIME_CODE,
  resolveZonedLocalTime,
  type FoldPolicy,
  type GapPolicy,
  type ZonedLocalTime,
} from './zoned-time.js';
import { zonedTimeVectorChecks, type ZonedTimeVectorFixture } from './zoned-time-vectors.js';

const fixtureUrl = new URL('../../../../fixtures/zoned-time/vectors-v1.json', import.meta.url);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixture = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as ZonedTimeVectorFixture;

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

  it.each(zonedTimeVectorChecks(fixture))('$label', ({ actual, expected }) => {
    expect(actual()).toEqual(expected);
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
