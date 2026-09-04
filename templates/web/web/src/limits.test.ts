import limitsFixture from '../../limits.json';
import { ResourceMeasurementError } from '@baukit/data-contracts/limits';
import { describe, expect, it } from 'vitest';

import {
  LIMITS_POLICY,
  LimitError,
  LimitsPolicyError,
  checkBatch,
  checkBody,
  checkCollection,
  checkJsonDocument,
  checkRows,
  checkText,
  parseLimitsPolicy,
} from './limits';

describe('shared limits policy', () => {
  it('loads the product-root fixture', () => {
    expect(LIMITS_POLICY).toEqual(limitsFixture);
  });

  it('rejects unknown versions, fields, and zero values', () => {
    expect(() => parseLimitsPolicy({ ...limitsFixture, version: 2 })).toThrow(LimitsPolicyError);
    expect(() => parseLimitsPolicy({ ...limitsFixture, extra: 1 })).toThrow(LimitsPolicyError);
    expect(() => parseLimitsPolicy({ ...limitsFixture, text: { max_characters: 0 } })).toThrow(
      LimitsPolicyError,
    );
  });

  it('accepts boundaries and reports every stable reason code', () => {
    expect(() => {
      checkText('title', 'é'.repeat(LIMITS_POLICY.text.max_characters));
    }).not.toThrow();
    expectReason(
      () => {
        checkText('title', 'é'.repeat(LIMITS_POLICY.text.max_characters + 1));
      },
      'text_too_long',
      'title',
    );
    expectReason(
      () => {
        checkJsonDocument('metadata', {
          value: 'x'.repeat(LIMITS_POLICY.document.max_bytes),
        });
      },
      'jsonb_too_large',
      'metadata',
    );
    expectReason(
      () => {
        checkCollection('entries', LIMITS_POLICY.collection.max_elements + 1);
      },
      'too_many_elements',
      'entries',
    );
    expectReason(
      () => {
        checkRows('records', LIMITS_POLICY.rows.max_count + 1);
      },
      'too_many_rows',
      'records',
    );
    expectReason(
      () => {
        checkBody('request', LIMITS_POLICY.body.max_bytes + 1);
      },
      'body_too_large',
      'request',
    );
    expectReason(
      () => {
        checkBatch('changes', LIMITS_POLICY.batch.max_items + 1);
      },
      'batch_too_large',
      'changes',
    );
  });

  it('rejects invalid counts instead of treating them as within policy', () => {
    expect(() => {
      checkRows('records', -1);
    }).toThrow(RangeError);
    expect(() => {
      checkBody('request', Number.NaN);
    }).toThrow(RangeError);
  });

  it('passes production measurement failures through without product content', () => {
    expect(() => {
      checkText('title', '\ud800');
    }).toThrow(ResourceMeasurementError);
    expect(() => {
      checkJsonDocument('metadata', Number.NaN);
    }).toThrow(ResourceMeasurementError);
  });
});

function expectReason(action: () => void, reason: LimitError['reason'], field: string): void {
  try {
    action();
    throw new Error('expected a limit error');
  } catch (error) {
    expect(error).toBeInstanceOf(LimitError);
    expect(error).toMatchObject({ reason, field });
    expect((error as Error).message).toBe(`Limit exceeded for ${field}: ${reason}`);
  }
}
