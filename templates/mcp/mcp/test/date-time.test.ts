import { describe, expect, it } from 'vitest';

import { dateTimeInput } from '../src/tools/date-time.js';

describe('date-time tool inputs', () => {
  it.each([
    '2026-10-03T09:30Z',
    '2026-10-03T09:30:00Z',
    '2026-10-03T09:30:00.123Z',
    '2026-10-03T11:30+02:00',
    '2026-10-03T11:30:00+02:00',
    '2026-10-03T04:00-05:30',
  ])('accepts the documented timestamp %s', (value) => {
    expect(dateTimeInput.parse(value)).toBe(value);
  });

  it.each([
    '2026-10-03T09:30',
    '2026-10-03',
    '2026-02-30T09:30Z',
    '2026-10-03T25:30Z',
    '2026-10-03T09:60Z',
    '2026-10-03T09:30+0200',
    '2026-10-03T09:30.123Z',
  ])('rejects an invalid or unqualified timestamp %s', (value) => {
    expect(dateTimeInput.safeParse(value).success).toBe(false);
  });
});
