import { afterEach, describe, expect, it, vi } from 'vitest';

import {
  addCivilDays,
  assertCivilDate,
  assertCivilTime,
  assertTimeZone,
  civilDateForInstant,
  civilDateValidationCode,
  civilDaysBetween,
  civilToday,
  compareCivilDates,
  INVALID_CIVIL_DATE_CODE,
  isCivilDate,
  isInstantOnCivilDate,
  parseCivilDate,
  resolvedTimeZone,
} from './civil-date.js';

afterEach(() => {
  vi.useRealTimers();
});

describe('parseCivilDate', () => {
  it('accepts a well-formed calendar date', () => {
    expect(parseCivilDate('2026-08-22')).toEqual({ ok: true, value: '2026-08-22' });
  });

  it('accepts February 29 in a leap year and rejects it otherwise', () => {
    expect(parseCivilDate('2024-02-29').ok).toBe(true);
    expect(parseCivilDate('2000-02-29').ok).toBe(true);
    expect(parseCivilDate('2026-02-29')).toEqual({ ok: false, code: INVALID_CIVIL_DATE_CODE });
    expect(parseCivilDate('1900-02-29')).toEqual({ ok: false, code: INVALID_CIVIL_DATE_CODE });
  });

  it.each([
    'not-a-date',
    '',
    '2026-02-30',
    '2026-13-01',
    '2026-00-10',
    '2026-01-00',
    '2026-1-01',
    '26-01-01',
    '2026/01/01',
    '2026-01-01T00:00:00Z',
    ' 2026-01-01',
    '2026-01-01 ',
    '+2026-01-01',
    '2026-04-31',
  ])('rejects %j', (candidate) => {
    expect(parseCivilDate(candidate)).toEqual({ ok: false, code: INVALID_CIVIL_DATE_CODE });
  });

  it('exposes boolean and code-shaped validation', () => {
    expect(isCivilDate('2026-08-22')).toBe(true);
    expect(isCivilDate('2026-02-30')).toBe(false);
    expect(civilDateValidationCode('2026-02-28')).toBeNull();
    expect(civilDateValidationCode('nope')).toBe(INVALID_CIVIL_DATE_CODE);
  });
});

describe('assertCivilDate', () => {
  it('throws on an impossible date and stays silent on a real one', () => {
    expect(() => {
      assertCivilDate('2026-02-30');
    }).toThrow(new RangeError('Invalid civil date: 2026-02-30'));
    expect(() => {
      assertCivilDate('2026-02-28');
    }).not.toThrow();
  });
});

describe('assertCivilTime', () => {
  it.each(['00:00', '23:59', '12:30:45', null])('accepts %j', (candidate) => {
    expect(() => {
      assertCivilTime(candidate);
    }).not.toThrow();
  });

  it.each(['24:00', '12:60', '7:30', '12:30:60', '', 'noon'])('rejects %j', (candidate) => {
    expect(() => {
      assertCivilTime(candidate);
    }).toThrow(RangeError);
  });
});

describe('assertTimeZone', () => {
  it('accepts real IANA zones and UTC', () => {
    for (const zone of ['UTC', 'Europe/Berlin', 'Pacific/Auckland', 'America/Los_Angeles']) {
      expect(() => {
        assertTimeZone(zone);
      }).not.toThrow();
    }
  });

  it('rejects empty and unknown zones', () => {
    expect(() => {
      assertTimeZone('   ');
    }).toThrow(new RangeError('Time zone must be non-empty'));
    expect(() => {
      assertTimeZone('Mars/Olympus_Mons');
    }).toThrow(new RangeError('Invalid IANA time zone: Mars/Olympus_Mons'));
  });
});

describe('addCivilDays', () => {
  it('crosses month and year boundaries', () => {
    expect(addCivilDays('2026-01-31', 1)).toBe('2026-02-01');
    expect(addCivilDays('2026-12-31', 1)).toBe('2027-01-01');
    expect(addCivilDays('2026-01-01', -1)).toBe('2025-12-31');
  });

  it('lands on February 29 in a leap year and skips it otherwise', () => {
    expect(addCivilDays('2024-02-28', 1)).toBe('2024-02-29');
    expect(addCivilDays('2026-02-28', 1)).toBe('2026-03-01');
    expect(addCivilDays('2024-02-29', 365)).toBe('2025-02-28');
  });

  it('ignores DST because civil days have no clock', () => {
    expect(addCivilDays('2026-03-28', 1)).toBe('2026-03-29');
    expect(addCivilDays('2026-10-24', 1)).toBe('2026-10-25');
    expect(addCivilDays('2026-03-07', 1)).toBe('2026-03-08');
  });

  it('returns the same date for zero days', () => {
    expect(addCivilDays('2026-08-22', 0)).toBe('2026-08-22');
  });

  it('rejects an invalid date and a non-integer offset', () => {
    expect(() => addCivilDays('2026-02-30', 1)).toThrow(RangeError);
    expect(() => addCivilDays('2026-02-28', 1.5)).toThrow(
      new RangeError('Days must be an integer'),
    );
    expect(() => addCivilDays('2026-02-28', Number.NaN)).toThrow(
      new RangeError('Days must be an integer'),
    );
  });

  it('rejects an offset that leaves the representable range', () => {
    expect(() => addCivilDays('2026-08-22', 400_000_000)).toThrow(RangeError);
  });
});

describe('civilDateForInstant', () => {
  it('splits an instant that is two different civil days in two zones', () => {
    const instant = '2026-08-24T10:00:00Z';
    expect(civilDateForInstant(instant, 'Pacific/Auckland')).toBe('2026-08-24');
    expect(civilDateForInstant(instant, 'America/Los_Angeles')).toBe('2026-08-24');

    const nightInstant = '2026-08-25T06:00:00Z';
    expect(civilDateForInstant(nightInstant, 'Pacific/Auckland')).toBe('2026-08-25');
    expect(civilDateForInstant(nightInstant, 'America/Los_Angeles')).toBe('2026-08-24');
  });

  it('resolves the civil day across a spring-forward gap', () => {
    expect(civilDateForInstant('2026-03-08T09:59:00Z', 'America/Los_Angeles')).toBe('2026-03-08');
    expect(civilDateForInstant('2026-03-08T10:00:00Z', 'America/Los_Angeles')).toBe('2026-03-08');
  });

  it('resolves both passes of a repeated fall-back hour to the same civil day', () => {
    expect(civilDateForInstant('2026-11-01T08:30:00Z', 'America/Los_Angeles')).toBe('2026-11-01');
    expect(civilDateForInstant('2026-11-01T09:30:00Z', 'America/Los_Angeles')).toBe('2026-11-01');
  });

  it('flips the civil day exactly at local midnight', () => {
    expect(civilDateForInstant('2026-08-21T21:59:59Z', 'Europe/Berlin')).toBe('2026-08-21');
    expect(civilDateForInstant('2026-08-21T22:00:00Z', 'Europe/Berlin')).toBe('2026-08-22');
  });

  it('accepts a Date, an ISO string, and epoch milliseconds', () => {
    expect(civilDateForInstant(new Date('2026-08-22T12:00:00Z'), 'UTC')).toBe('2026-08-22');
    expect(civilDateForInstant('2026-08-22T12:00:00Z', 'UTC')).toBe('2026-08-22');
    expect(civilDateForInstant(Date.parse('2026-08-22T12:00:00Z'), 'UTC')).toBe('2026-08-22');
  });

  it('zero-pads single-digit months and days', () => {
    expect(civilDateForInstant('2026-01-05T12:00:00Z', 'UTC')).toBe('2026-01-05');
  });

  it('rejects an unparsable instant and an unknown zone', () => {
    expect(() => civilDateForInstant('yesterday', 'UTC')).toThrow(
      new RangeError('Invalid instant: yesterday'),
    );
    expect(() => civilDateForInstant(Number.NaN, 'UTC')).toThrow(RangeError);
    expect(() => civilDateForInstant('2026-08-22T12:00:00Z', 'Mars/Olympus_Mons')).toThrow(
      RangeError,
    );
  });
});

describe('civilToday', () => {
  it('reads the current instant in the requested zone', () => {
    vi.useFakeTimers();
    vi.setSystemTime(new Date('2026-08-24T23:30:00Z'));
    expect(civilToday('UTC')).toBe('2026-08-24');
    expect(civilToday('Pacific/Auckland')).toBe('2026-08-25');
    expect(civilToday('America/Los_Angeles')).toBe('2026-08-24');
  });
});

describe('resolvedTimeZone', () => {
  it('returns a zone the assertion accepts', () => {
    const zone = resolvedTimeZone();
    expect(typeof zone).toBe('string');
    expect(() => {
      assertTimeZone(zone);
    }).not.toThrow();
  });
});

describe('isInstantOnCivilDate', () => {
  it('answers differently per zone for one instant', () => {
    const instant = '2026-08-25T06:00:00Z';
    expect(isInstantOnCivilDate(instant, '2026-08-25', 'Pacific/Auckland')).toBe(true);
    expect(isInstantOnCivilDate(instant, '2026-08-25', 'America/Los_Angeles')).toBe(false);
    expect(isInstantOnCivilDate(instant, '2026-08-24', 'America/Los_Angeles')).toBe(true);
  });

  it('rejects an invalid civil day', () => {
    expect(() => isInstantOnCivilDate('2026-08-22T12:00:00Z', '2026-02-30', 'UTC')).toThrow(
      RangeError,
    );
  });
});

describe('compareCivilDates', () => {
  it('orders civil dates and reports equality', () => {
    expect(compareCivilDates('2026-08-22', '2026-08-23')).toBe(-1);
    expect(compareCivilDates('2026-08-23', '2026-08-22')).toBe(1);
    expect(compareCivilDates('2026-08-22', '2026-08-22')).toBe(0);
    expect(compareCivilDates('2025-12-31', '2026-01-01')).toBe(-1);
  });

  it('rejects an invalid operand', () => {
    expect(() => compareCivilDates('2026-02-30', '2026-03-01')).toThrow(RangeError);
    expect(() => compareCivilDates('2026-03-01', 'nope')).toThrow(RangeError);
  });
});

describe('civilDaysBetween', () => {
  it('counts whole days including across DST and leap days', () => {
    expect(civilDaysBetween('2026-08-22', '2026-08-25')).toBe(3);
    expect(civilDaysBetween('2026-08-25', '2026-08-22')).toBe(-3);
    expect(civilDaysBetween('2026-08-22', '2026-08-22')).toBe(0);
    expect(civilDaysBetween('2026-03-07', '2026-03-09')).toBe(2);
    expect(civilDaysBetween('2024-02-28', '2024-03-01')).toBe(2);
    expect(civilDaysBetween('2026-02-28', '2026-03-01')).toBe(1);
  });

  it('rejects an invalid operand', () => {
    expect(() => civilDaysBetween('2026-02-30', '2026-03-01')).toThrow(RangeError);
  });
});
