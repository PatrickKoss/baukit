import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import { fullJitterDelayMs } from './scheduler-policy.js';

const fixtureUrl = new URL('../../../../fixtures/resilience/backoff-v1.json', import.meta.url);

interface FullJitterVector {
  readonly initialMs: number;
  readonly maxMs: number;
  readonly attempt: number;
  readonly fraction: number;
  readonly expectedDelayMs: number;
}

function numberField(record: object, key: string): number {
  const value: unknown = Reflect.get(record, key);
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    throw new Error(`${key} must be a number`);
  }
  return value;
}

function fullJitterVectors(): FullJitterVector[] {
  const parsed: unknown = JSON.parse(readFileSync(fixtureUrl, 'utf8'));
  const cases: unknown =
    typeof parsed === 'object' && parsed !== null ? Reflect.get(parsed, 'cases') : null;
  if (!Array.isArray(cases)) {
    throw new Error('backoff-v1.json has no cases');
  }
  return cases
    .filter((entry): entry is object => typeof entry === 'object' && entry !== null)
    .filter((entry) => Reflect.get(entry, 'strategy') === 'full_jitter')
    .map((entry) => ({
      initialMs: numberField(entry, 'initial_ms'),
      maxMs: numberField(entry, 'max_ms'),
      attempt: numberField(entry, 'attempt'),
      fraction: numberField(entry, 'fraction'),
      expectedDelayMs: numberField(entry, 'expected_delay_ms'),
    }));
}

describe('fullJitterDelayMs against backoff-v1.json', () => {
  const vectors = fullJitterVectors();

  it('loads the full jitter cases', () => {
    expect(vectors.length).toBeGreaterThan(0);
  });

  it.each(vectors)('case %#', (vector) => {
    const delay = fullJitterDelayMs(vector.attempt, () => vector.fraction, {
      baseDelayMs: vector.initialMs,
      maxDelayMs: vector.maxMs,
    });

    expect(delay).toBe(vector.expectedDelayMs);
  });

  it('uses the 1 s to 60 s default bounds', () => {
    const defaults = vectors.filter(
      (vector) => vector.initialMs === 1_000 && vector.maxMs === 60_000,
    );

    expect(defaults.length).toBeGreaterThan(0);
    for (const vector of defaults) {
      expect(fullJitterDelayMs(vector.attempt, () => vector.fraction)).toBe(vector.expectedDelayMs);
    }
  });
});
