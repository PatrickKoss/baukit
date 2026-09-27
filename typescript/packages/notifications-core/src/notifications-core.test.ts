import { describe, expect, it } from 'vitest';

import { NotificationPlanError } from './errors.js';
import { InMemoryNotificationPlatform } from './memory.js';
import { resolveNotificationOccurrences } from './occurrences.js';
import {
  decodeOwnedNotificationMarker,
  encodeOwnedNotificationMarker,
  isOwnedBy,
  ownedNotificationIdentifier,
  type OwnedNotificationMarker,
} from './ownership.js';
import { planNotificationReplacement } from './replacement-plan.js';
import { createOwnedNotificationScheduler } from './scheduler.js';
import { describeOwnedNotificationSchedulerContract } from './vitest.js';

const NOW = Date.parse('2026-06-15T06:00:00Z');
const MARKER: OwnedNotificationMarker = {
  namespace: 'reminders',
  logicalId: 'plan:2026-06-15',
  epochMilliseconds: NOW,
  contentDigest: 'sha256:abc',
};

const baseInput = {
  timeZone: 'Europe/Berlin',
  gap: 'reject',
  fold: 'earlier',
  clock: () => NOW,
  horizonDays: 2,
} as const;

function markerJson(overrides: Record<string, unknown>): string {
  return JSON.stringify({ version: 1, ...MARKER, ...overrides });
}

describe('resolveNotificationOccurrences', () => {
  it('carries product fields through to the desired set', () => {
    const resolution = resolveNotificationOccurrences({
      ...baseInput,
      occurrences: [
        {
          logicalId: 'a',
          civilDate: '2026-06-15',
          civilTime: '09:00',
          contentDigest: 'v1',
          title: 'kept',
        },
      ],
    });
    expect(resolution.desired).toEqual([
      {
        logicalId: 'a',
        civilDate: '2026-06-15',
        civilTime: '09:00',
        contentDigest: 'v1',
        title: 'kept',
        epochMilliseconds: Date.parse('2026-06-15T07:00:00Z'),
        transition: 'none',
      },
    ]);
  });

  it('rejects an unknown policy even without occurrences', () => {
    expect(() =>
      resolveNotificationOccurrences({
        ...baseInput,
        gap: 'skip' as 'reject',
        occurrences: [],
      }),
    ).toThrow(RangeError);
  });

  it('returns the same result for repeated calls with the same clock', () => {
    const input = {
      ...baseInput,
      occurrences: [
        { logicalId: 'b', civilDate: '2026-06-16', civilTime: '09:00', contentDigest: 'v1' },
        { logicalId: 'a', civilDate: '2026-06-15', civilTime: '09:00', contentDigest: 'v1' },
      ],
    };
    expect(resolveNotificationOccurrences(input)).toEqual(resolveNotificationOccurrences(input));
  });
});

describe('planNotificationReplacement', () => {
  it('keeps an entry whose text changed under the same digest', () => {
    const current = [{ logicalId: 'a', epochMilliseconds: NOW + 1, contentDigest: 'v1' }];
    const desired = [
      { logicalId: 'a', epochMilliseconds: NOW + 1, contentDigest: 'v1', text: 'new' },
    ];
    expect(planNotificationReplacement(current, desired)).toEqual({
      keep: current,
      cancel: [],
      schedule: [],
    });
  });

  it('rejects an instant that is not a safe integer', () => {
    const desired = [{ logicalId: 'a', epochMilliseconds: 1.5, contentDigest: 'v1' }];
    expect(() => planNotificationReplacement([], desired)).toThrow(
      expect.objectContaining({ code: 'invalid_instant', logicalId: 'a' }),
    );
  });
});

describe('owned notification markers', () => {
  it('round-trips a marker', () => {
    expect(decodeOwnedNotificationMarker(encodeOwnedNotificationMarker(MARKER))).toEqual(MARKER);
  });

  it.each([
    ['an object', { ...MARKER }],
    ['malformed JSON', '{'],
    ['JSON null', 'null'],
    ['another version', markerJson({ version: 2 })],
    ['an invalid namespace', markerJson({ namespace: 'Upper' })],
    ['an empty logical ID', markerJson({ logicalId: '' })],
    ['a string instant', markerJson({ epochMilliseconds: '1' })],
    ['a missing digest', markerJson({ contentDigest: undefined })],
  ])('ignores %s', (_label, value) => {
    expect(decodeOwnedNotificationMarker(value)).toBeNull();
  });

  it('owns a request only when the identifier matches the marker', () => {
    const identifier = ownedNotificationIdentifier(MARKER.namespace, MARKER.logicalId);
    expect(identifier).toBe('baukit:reminders:plan:2026-06-15');
    expect(isOwnedBy('reminders', identifier, MARKER)).toBe(true);
    expect(isOwnedBy('reminders', 'forged', MARKER)).toBe(false);
    expect(isOwnedBy('reminders-extra', identifier, MARKER)).toBe(false);
    expect(isOwnedBy('reminders', identifier, null)).toBe(false);
  });

  it('rejects an invalid namespace in identifiers', () => {
    expect(() => ownedNotificationIdentifier('a:b', 'x')).toThrow(NotificationPlanError);
  });
});

describe('createOwnedNotificationScheduler', () => {
  it.each([0, -1, 1.5, Number.NaN])('rejects a pending limit of %s', (pendingLimit) => {
    const platform = new InMemoryNotificationPlatform<string>();
    expect(() => createOwnedNotificationScheduler(platform, { pendingLimit })).toThrow(
      expect.objectContaining({ code: 'invalid_pending_limit' }),
    );
  });
});

describe('InMemoryNotificationPlatform', () => {
  describeOwnedNotificationSchedulerContract<string>((options) => {
    const platform = new InMemoryNotificationPlatform<string>();
    return {
      scheduler: createOwnedNotificationScheduler(platform, options),
      faults: platform.faults,
      content: (_logicalId, text) => text,
      addUnrelated: (identifier) => {
        platform.addUnrelated(identifier);
      },
      pending: () => platform.pending(),
    };
  });
});
