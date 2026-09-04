import limitsFixture from '../../limits.json';
import {
  LimitExceededError,
  checkCompactJsonUtf8Bytes,
  checkMeasurement,
  checkTrimmedUnicodeScalars,
} from '@baukit/data-contracts/limits';

const SUPPORTED_POLICY_VERSION = 1;

export interface LimitsPolicy {
  readonly $comment: string;
  readonly version: number;
  readonly text: { readonly max_characters: number };
  readonly collection: { readonly max_elements: number };
  readonly document: { readonly max_bytes: number };
  readonly rows: { readonly max_count: number };
  readonly body: { readonly max_bytes: number };
  readonly batch: { readonly max_items: number };
}

export type LimitReason =
  | 'text_too_long'
  | 'jsonb_too_large'
  | 'too_many_elements'
  | 'too_many_rows'
  | 'body_too_large'
  | 'batch_too_large';

export type JsonValue =
  null | boolean | number | string | readonly JsonValue[] | { readonly [key: string]: JsonValue };

export class LimitsPolicyError extends Error {}

export class LimitError extends Error {
  constructor(
    readonly reason: LimitReason,
    readonly field: string,
  ) {
    super(`Limit exceeded for ${field}: ${reason}`);
    this.name = 'LimitError';
  }
}

export function parseLimitsPolicy(value: unknown): LimitsPolicy {
  const policy = expectObject(value, 'limits');
  expectKeys(
    policy,
    ['$comment', 'version', 'text', 'collection', 'document', 'rows', 'body', 'batch'],
    'limits',
  );
  if (typeof policy['$comment'] !== 'string') {
    throw new LimitsPolicyError('limits.$comment must be a string');
  }
  if (policy['version'] !== SUPPORTED_POLICY_VERSION) {
    throw new LimitsPolicyError(`Unsupported limits policy version ${String(policy['version'])}`);
  }
  checkSection(policy, 'text', 'max_characters');
  checkSection(policy, 'collection', 'max_elements');
  checkSection(policy, 'document', 'max_bytes');
  checkSection(policy, 'rows', 'max_count');
  checkSection(policy, 'body', 'max_bytes');
  checkSection(policy, 'batch', 'max_items');
  return policy as unknown as LimitsPolicy;
}

export const LIMITS_POLICY = parseLimitsPolicy(limitsFixture);

export function checkText(field: string, value: string): void {
  mapLimitExceeded(field, 'text_too_long', () =>
    checkTrimmedUnicodeScalars(value, LIMITS_POLICY.text.max_characters),
  );
}

export function checkJsonDocument(field: string, value: JsonValue): void {
  mapLimitExceeded(field, 'jsonb_too_large', () =>
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

function expectObject(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new LimitsPolicyError(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function expectKeys(
  value: Record<string, unknown>,
  expected: readonly string[],
  path: string,
): void {
  const actual = Object.keys(value).sort();
  const wanted = [...expected].sort();
  if (actual.length !== wanted.length || actual.some((key, index) => key !== wanted[index])) {
    throw new LimitsPolicyError(`${path} has unknown or missing fields`);
  }
}

function checkSection(
  policy: Record<string, unknown>,
  sectionName: string,
  valueName: string,
): void {
  const section = expectObject(policy[sectionName], `limits.${sectionName}`);
  expectKeys(section, [valueName], `limits.${sectionName}`);
  const value = section[valueName];
  if (!Number.isSafeInteger(value) || (value as number) <= 0) {
    throw new LimitsPolicyError(`limits.${sectionName}.${valueName} must be a positive integer`);
  }
}

function checkCount(field: string, actual: number, maximum: number, reason: LimitReason): void {
  if (!Number.isSafeInteger(actual) || actual < 0) {
    throw new RangeError(`${field} count must be a non-negative integer`);
  }
  mapLimitExceeded(field, reason, () => checkMeasurement(actual, maximum));
}

function mapLimitExceeded(field: string, reason: LimitReason, check: () => unknown): void {
  try {
    check();
  } catch (error) {
    if (error instanceof LimitExceededError) throw new LimitError(reason, field);
    throw error;
  }
}
