import { describe, expect, it } from 'vitest';

import { fullJitterBackoffMs } from './backoff.js';

const policy = { baseDelayMs: 100, maxDelayMs: 1_000 };

describe('fullJitterBackoffMs', () => {
  it('doubles the ceiling per retry and caps it', () => {
    const top = { ...policy, random: () => 1 };

    expect([0, 1, 2, 3, 4, 10].map((index) => fullJitterBackoffMs(index, top))).toEqual([
      100, 200, 400, 800, 1_000, 1_000,
    ]);
  });

  it('scales the ceiling by the random sample', () => {
    expect(fullJitterBackoffMs(2, { ...policy, random: () => 0.25 })).toBe(100);
    expect(fullJitterBackoffMs(2, { ...policy, random: () => 0 })).toBe(0);
  });

  it('clamps samples outside zero to one', () => {
    expect(fullJitterBackoffMs(0, { ...policy, random: () => 2 })).toBe(100);
    expect(fullJitterBackoffMs(0, { ...policy, random: () => -1 })).toBe(0);
  });

  it('defaults to Math.random within the ceiling', () => {
    const delay = fullJitterBackoffMs(1, policy);

    expect(delay).toBeGreaterThanOrEqual(0);
    expect(delay).toBeLessThanOrEqual(200);
  });
});
