import { readFileSync } from 'node:fs';

import { isNonNegativeSafeInteger, isRecord, isString } from '@baukit/type-guards';
import { describe, expect, it } from 'vitest';

import { fullJitterBackoffMs } from './backoff.js';
import { OriginCooldown } from './cooldown.js';

const fixtureRoot = new URL('../../../../fixtures/resilience/', import.meta.url);
const ORIGIN = 'https://api.example.test';

/** Scenarios whose closed and open phases the cooldown models; it has no half-open probe. */
const COOLDOWN_SCENARIOS = [
  'consecutive timeouts open the breaker',
  'retry-after holds the breaker without counting a failure',
];

function readFixture(name: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(new URL(name, fixtureRoot), 'utf8'));
  if (!isRecord(parsed) || parsed['version'] !== 1) {
    throw new Error(`${name} is not a version 1 fixture`);
  }
  return parsed;
}

function records(value: unknown): Record<string, unknown>[] {
  if (!Array.isArray(value) || !value.every(isRecord)) {
    throw new Error('expected a list of objects');
  }
  return value;
}

function integer(record: Record<string, unknown>, key: string): number {
  const value = record[key];
  if (!isNonNegativeSafeInteger(value)) {
    throw new Error(`${key} must be a non-negative integer`);
  }
  return value;
}

function fraction(record: Record<string, unknown>): number {
  const value = record['fraction'];
  if (typeof value !== 'number' || value < 0 || value > 1) {
    throw new Error('fraction must be in [0, 1]');
  }
  return value;
}

function text(record: Record<string, unknown>, key: string): string {
  const value = record[key];
  if (!isString(value)) {
    throw new Error(`${key} must be a string`);
  }
  return value;
}

describe('backoff-v1.json', () => {
  const cases = records(readFixture('backoff-v1.json')['cases']).filter(
    (vector) => vector['strategy'] === 'full_jitter',
  );

  it.each(cases)('full jitter case %#', (vector) => {
    const delay = fullJitterBackoffMs(integer(vector, 'attempt'), {
      baseDelayMs: integer(vector, 'initial_ms'),
      maxDelayMs: integer(vector, 'max_ms'),
      random: () => fraction(vector),
    });

    expect(delay).toBe(integer(vector, 'expected_delay_ms'));
  });
});

describe('breaker-v1.json against the origin cooldown', () => {
  const fixture = readFixture('breaker-v1.json');
  const policy = isRecord(fixture['policy']) ? fixture['policy'] : {};
  const scenarios = records(fixture['scenarios']).filter((scenario) =>
    COOLDOWN_SCENARIOS.includes(text(scenario, 'name')),
  );

  it('finds every scenario it replays', () => {
    expect(scenarios).toHaveLength(COOLDOWN_SCENARIOS.length);
  });

  it.each(scenarios)('$name', (scenario) => {
    let nowMs = 0;
    let jitter = 0;
    const cooldown = new OriginCooldown(
      {
        networkFailureThreshold: integer(policy, 'consecutive_timeouts'),
        networkFailureCooldownMs: integer(policy, 'open_base_ms'),
        maxCooldownMs: integer(policy, 'open_max_ms'),
        jitterPercent: integer(policy, 'jitter_percent'),
        random: () => jitter,
      },
      () => nowMs,
    );

    for (const event of records(scenario['events'])) {
      nowMs = integer(event, 'at_ms');
      if (event['op'] === 'record') {
        jitter = fraction(event);
        applyOutcome(cooldown, event);
      }
      const blockedUntil = cooldown.blockedUntil(ORIGIN);
      if (text(event, 'expect_state') !== 'open') {
        expect(blockedUntil, `at ${String(nowMs)}`).toBeNull();
        continue;
      }
      expect(blockedUntil, `at ${String(nowMs)}`).not.toBeNull();
      if (event['expect_retry_after_ms'] !== undefined) {
        expect((blockedUntil ?? 0) - nowMs).toBe(integer(event, 'expect_retry_after_ms'));
      }
    }
  });
});

function applyOutcome(cooldown: OriginCooldown, event: Record<string, unknown>): void {
  const outcome = text(event, 'outcome');
  if (outcome === 'timeout') {
    cooldown.recordNetworkFailure(ORIGIN);
  } else if (outcome === 'success') {
    cooldown.recordResponse(ORIGIN);
  } else if (outcome === 'hold_for') {
    cooldown.coolDown(ORIGIN, integer(event, 'hold_for_ms'));
  } else if (outcome !== 'ignored') {
    throw new Error(`outcome ${outcome} is outside the cooldown model`);
  }
}
