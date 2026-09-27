import limitsFixture from '../../limits.json';
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
  checkText,
  type LimitReason,
} from './limits';

describe('limits policy call site', () => {
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
