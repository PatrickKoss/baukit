import { describe, expect, it } from 'vitest';

import { OriginCooldown } from './cooldown.js';

const ORIGIN = 'https://api.example.test';

function failRun(cooldown: OriginCooldown, count: number): void {
  for (let index = 0; index < count; index += 1) {
    cooldown.recordNetworkFailure(ORIGIN);
  }
}

describe('OriginCooldown', () => {
  it('doubles repeated network cooldowns up to the cap until a response arrives', () => {
    let nowMs = 0;
    const cooldown = new OriginCooldown(
      {
        networkFailureThreshold: 1,
        networkFailureCooldownMs: 1_000,
        maxCooldownMs: 3_000,
        random: () => 0,
      },
      () => nowMs,
    );

    const ends = [1_000, 2_000, 3_000, 3_000].map(() => {
      failRun(cooldown, 1);
      const end = cooldown.blockedUntil(ORIGIN) ?? 0;
      const length = end - nowMs;
      nowMs = end;
      return length;
    });
    expect(ends).toEqual([1_000, 2_000, 3_000, 3_000]);

    cooldown.recordResponse(ORIGIN);
    failRun(cooldown, 1);
    expect(cooldown.blockedUntil(ORIGIN)).toBe(nowMs + 1_000);
  });

  it('takes the jitter share off the network cooldown', () => {
    const cooldown = new OriginCooldown(
      { networkFailureThreshold: 1, jitterPercent: 20, random: () => 0.5 },
      () => 0,
    );

    failRun(cooldown, 1);

    expect(cooldown.blockedUntil(ORIGIN)).toBe(27_000);
  });

  it('caps a Retry-After hold', () => {
    const cooldown = new OriginCooldown({ maxCooldownMs: 5_000 }, () => 0);

    cooldown.coolDown(ORIGIN, 60_000);

    expect(cooldown.blockedUntil(ORIGIN)).toBe(5_000);
  });

  it('rejects a jitter share above 100 percent', () => {
    expect(() => new OriginCooldown({ jitterPercent: 101 }, () => 0)).toThrow(/jitterPercent/);
  });
});
