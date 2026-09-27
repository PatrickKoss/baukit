import limitsFixture from '../../limits.json';
import {
  checkCompactJsonUtf8Bytes,
  checkMeasurement,
  checkTrimmedUnicodeScalars,
  enforceLimit,
  parseLimitsPolicy,
  type LimitsPolicy as SchemaLimitsPolicy,
} from '@baukit/data-contracts/limits';

export const LIMITS_POLICY_SCHEMA = {
  version: 1,
  sections: {
    text: ['max_characters'],
    collection: ['max_elements'],
    document: ['max_bytes'],
    rows: ['max_count'],
    body: ['max_bytes'],
    batch: ['max_items'],
  },
} as const;

export type LimitsPolicy = SchemaLimitsPolicy<typeof LIMITS_POLICY_SCHEMA>;

export type LimitReason =
  | 'text_too_long'
  | 'jsonb_too_large'
  | 'too_many_elements'
  | 'too_many_rows'
  | 'body_too_large'
  | 'batch_too_large';

export type JsonValue =
  null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

export const LIMITS_POLICY: LimitsPolicy = parseLimitsPolicy(limitsFixture, LIMITS_POLICY_SCHEMA);

export function checkText(field: string, value: string): void {
  enforceLimit(field, 'text_too_long', () =>
    checkTrimmedUnicodeScalars(value, LIMITS_POLICY.text.max_characters),
  );
}

export function checkJsonDocument(field: string, value: JsonValue): void {
  enforceLimit(field, 'jsonb_too_large', () =>
    checkCompactJsonUtf8Bytes(value, LIMITS_POLICY.document.max_bytes),
  );
}

export function checkCollection(field: string, count: number): void {
  checkCount(field, count, LIMITS_POLICY.collection.max_elements, 'too_many_elements');
}

export function checkRows(field: string, count: number): void {
  checkCount(field, count, LIMITS_POLICY.rows.max_count, 'too_many_rows');
}

export function checkBody(field: string, byteLength: number): void {
  checkCount(field, byteLength, LIMITS_POLICY.body.max_bytes, 'body_too_large');
}

export function checkBatch(field: string, count: number): void {
  checkCount(field, count, LIMITS_POLICY.batch.max_items, 'batch_too_large');
}

function checkCount(field: string, count: number, allowed: number, reason: LimitReason): void {
  enforceLimit(field, reason, () => checkMeasurement(count, allowed));
}
