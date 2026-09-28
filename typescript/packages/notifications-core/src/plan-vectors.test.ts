import { readFileSync } from 'node:fs';

import { describe, expect, it } from 'vitest';

import {
  notificationPlanVectorChecks,
  type NotificationPlanVectorFixture,
} from './plan-vectors.js';

const fixtureUrl = new URL(
  '../../../../fixtures/notifications/plan-vectors-v1.json',
  import.meta.url,
);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const fixture = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as NotificationPlanVectorFixture;

describe('notification plan vectors', () => {
  it('reads version 1 of the shared vectors', () => {
    expect(fixture.version).toBe(1);
    const errorCases = fixture.cases.filter((entry) => 'error' in entry.expected);
    expect(errorCases.length).toBeGreaterThan(0);
    expect(errorCases.length).toBeLessThan(fixture.cases.length);
  });

  it.each(notificationPlanVectorChecks(fixture))('$label', ({ actual, expected }) => {
    expect(actual()).toEqual(expected);
  });
});
