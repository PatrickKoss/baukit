import { describe, expect, expectTypeOf, it } from 'vitest';

import {
  LimitError,
  LimitExceededError,
  LimitsPolicyError,
  checkMeasurement,
  checkTrimmedUnicodeScalars,
  enforceLimit,
  parseLimitsPolicy,
} from './limits.js';

const schema = {
  version: 1,
  sections: {
    text: ['max_characters'],
    transport: ['max_bytes', 'daily_budget'],
  },
  allowZero: ['transport.daily_budget'],
} as const;

const policy = {
  $comment: 'Reviewed limits.',
  version: 1,
  text: { max_characters: 200 },
  transport: { max_bytes: 1024, daily_budget: 0 },
};

describe('parseLimitsPolicy', () => {
  it('returns the policy typed by the schema', () => {
    const parsed = parseLimitsPolicy(policy, schema);

    expect(parsed).toEqual(policy);
    expectTypeOf(parsed.text.max_characters).toEqualTypeOf<number>();
    expectTypeOf(parsed.transport.daily_budget).toEqualTypeOf<number>();
    expectTypeOf(parsed).not.toHaveProperty('rows');
    expectTypeOf(parsed.text).not.toHaveProperty('max_bytes');
  });

  it('rejects values that are not objects', () => {
    for (const value of [null, [], 'limits', 1]) {
      expectPolicyError(() => parseLimitsPolicy(value, schema), 'limits must be an object');
    }
  });

  it('rejects unknown and missing top-level fields', () => {
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, extra: 1 }, schema),
      'limits has unknown or missing fields',
    );
    const withoutComment = Object.fromEntries(
      Object.entries(policy).filter(([key]) => key !== '$comment'),
    );
    expectPolicyError(
      () => parseLimitsPolicy(withoutComment, schema),
      'limits has unknown or missing fields',
    );
  });

  it('requires a string comment and the schema version', () => {
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, $comment: 1 }, schema),
      'limits.$comment must be a string',
    );
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, version: 2 }, schema),
      'Unsupported limits policy version 2',
    );
  });

  it('rejects malformed sections and unknown section keys', () => {
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, text: null }, schema),
      'limits.text must be an object',
    );
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, text: { max_characters: 1, extra: 1 } }, schema),
      'limits.text has unknown or missing fields',
    );
  });

  it('requires positive safe integers unless the schema allows zero', () => {
    for (const limit of [0, -1, 1.5, Number.MAX_SAFE_INTEGER + 1, '200']) {
      expectPolicyError(
        () => parseLimitsPolicy({ ...policy, text: { max_characters: limit } }, schema),
        'limits.text.max_characters must be a positive integer',
      );
    }
    expectPolicyError(
      () => parseLimitsPolicy({ ...policy, transport: { max_bytes: 1, daily_budget: -1 } }, schema),
      'limits.transport.daily_budget must be a non-negative integer',
    );
  });

  it('rejects schemas that shadow reserved fields or allow zero on unknown keys', () => {
    expect(() => parseLimitsPolicy(policy, { version: 1, sections: { version: ['max'] } })).toThrow(
      TypeError,
    );
    expect(() => parseLimitsPolicy(policy, { ...schema, allowZero: ['text.max_bytes'] })).toThrow(
      TypeError,
    );
  });
});

describe('enforceLimit', () => {
  it('returns the measurement when the value fits', () => {
    expect(
      enforceLimit('title', 'text_too_long', () => checkTrimmedUnicodeScalars(' ab ', 2)),
    ).toEqual({
      measured: 2,
      allowed: 2,
    });
  });

  it('maps an exceeded limit to a LimitError with reason, field, and measurement', () => {
    let caught: unknown;
    try {
      enforceLimit('title', 'text_too_long', () => checkTrimmedUnicodeScalars('abc', 2));
    } catch (error) {
      caught = error;
    }

    expect(caught).toBeInstanceOf(LimitError);
    expect(caught).not.toBeInstanceOf(LimitExceededError);
    expect(caught).toMatchObject({
      name: 'LimitError',
      reason: 'text_too_long',
      field: 'title',
      measured: 3,
      allowed: 2,
      message: 'Limit exceeded for title: text_too_long',
    });
  });

  it('passes other failures through', () => {
    expect(() => enforceLimit('rows', 'too_many_rows', () => checkMeasurement(-1, 2))).toThrow(
      RangeError,
    );
  });
});

function expectPolicyError(action: () => unknown, message: string): void {
  expect(action).toThrow(LimitsPolicyError);
  expect(action).toThrow(message);
}
