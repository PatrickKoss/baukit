import limitsFixture from '../../limits.json';
import fixtureCorpus from '../../tests/fixtures/resource-budget-measurements-v1.json';
import {
  LimitError,
  LimitsPolicyError,
  ResourceMeasurementError,
  parseLimitsPolicy,
} from '@baukit/data-contracts/limits';

import {
  LIMITS_POLICY,
  LIMITS_POLICY_SCHEMA,
  checkBatch,
  checkBody,
  checkCollection,
  checkJsonDocument,
  checkRows,
  checkRange,
  checkText,
  type LimitReason,
} from './limits';

describe('limits policy call site', () => {
  it.each(fixtureCorpus.ranges)('checks range: $name', ({ measured, minimum, maximum, outcome }) => {
    if (outcome === 'ok') {
      expect(() => {
        checkRange('intervalSeconds', measured, minimum, maximum);
      }).not.toThrow();
      return;
    }
    if (outcome === 'invalid_bounds') {
      expect(() => {
        checkRange('intervalSeconds', measured, minimum, maximum);
      }).toThrow(RangeError);
      return;
    }
    expect(outcome).toBe('out_of_range');
    let caught: unknown;
    try {
      checkRange('intervalSeconds', measured, minimum, maximum);
    } catch (error) {
      caught = error;
    }
    expect(caught).toBeInstanceOf(LimitError);
    expect(caught).toMatchObject({ reason: 'out_of_range', field: 'intervalSeconds', measured, minimum, maximum });
  });

  it.each(fixtureCorpus.range_policies)('parses range: $name', ({ bounds, valid }) => {
    const value = { ...limitsFixture, ranges: { interval_seconds: bounds } };
    if (valid) {
      expect(parseLimitsPolicy(value, LIMITS_POLICY_SCHEMA).ranges.interval_seconds).toEqual(bounds);
    } else {
      expect(() => parseLimitsPolicy(value, LIMITS_POLICY_SCHEMA)).toThrow(LimitsPolicyError);
    }
  });

  it('checks the range declared in the shared policy', () => {
    const { minimum, maximum } = LIMITS_POLICY.ranges.interval_seconds;
    expect(() => {
      checkRange('intervalSeconds', minimum, minimum, maximum);
    }).not.toThrow();
    expect(() => {
      checkRange('intervalSeconds', maximum + 1, minimum, maximum);
    }).toThrow(LimitError);
  });

  it('loads the product-root fixture', () => {
    expect(LIMITS_POLICY).toEqual(limitsFixture);
  });

  it('parses the fixture against the template schema', () => {
    const parse = (value: unknown) => parseLimitsPolicy(value, LIMITS_POLICY_SCHEMA);
    expect(() => parse({ ...limitsFixture, version: 2 })).toThrow(LimitsPolicyError);
    expect(() => parse({ ...limitsFixture, extra: 1 })).toThrow(LimitsPolicyError);
  });

  it('reports every stable reason code', () => {
    expect(() => {
      checkText('title', 'é'.repeat(LIMITS_POLICY.text.max_characters));
    }).not.toThrow();
    expectReason(() => {
      checkText('title', 'é'.repeat(LIMITS_POLICY.text.max_characters + 1));
    }, 'text_too_long');
    expectReason(() => {
      checkJsonDocument('title', { value: 'x'.repeat(LIMITS_POLICY.document.max_bytes) });
    }, 'jsonb_too_large');
    expectReason(() => {
      checkCollection('title', LIMITS_POLICY.collection.max_elements + 1);
    }, 'too_many_elements');
    expectReason(() => {
      checkRows('title', LIMITS_POLICY.rows.max_count + 1);
    }, 'too_many_rows');
    expectReason(() => {
      checkBody('title', LIMITS_POLICY.body.max_bytes + 1);
    }, 'body_too_large');
    expectReason(() => {
      checkBatch('title', LIMITS_POLICY.batch.max_items + 1);
    }, 'batch_too_large');
  });

  it('rejects invalid counts and passes measurement failures through', () => {
    expect(() => {
      checkRows('records', -1);
    }).toThrow(RangeError);
    expect(() => {
      checkText('title', '\ud800');
    }).toThrow(ResourceMeasurementError);
  });
});

function expectReason(action: () => void, reason: LimitReason): void {
  expect(action).toThrow(LimitError);
  expect(action).toThrow(`Limit exceeded for title: ${reason}`);
}
