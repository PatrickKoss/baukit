export interface LimitMeasurement {
  readonly measured: number;
  readonly allowed: number;
}

export class LimitExceededError extends Error implements LimitMeasurement {
  readonly measured: number;
  readonly allowed: number;

  constructor(measured: number, allowed: number) {
    super(`Measured ${String(measured)} exceeds allowed ${String(allowed)}`);
    this.name = 'LimitExceededError';
    this.measured = measured;
    this.allowed = allowed;
  }
}

export interface LimitBounds {
  readonly minimum: number;
  readonly maximum: number;
}

export interface LimitRange extends LimitBounds {
  readonly measured: number;
}

export class LimitOutOfRangeError extends Error implements LimitRange {
  readonly measured: number;
  readonly minimum: number;
  readonly maximum: number;

  constructor(measured: number, minimum: number, maximum: number) {
    super(`Measured ${String(measured)} is outside ${String(minimum)}..=${String(maximum)}`);
    this.name = 'LimitOutOfRangeError';
    this.measured = measured;
    this.minimum = minimum;
    this.maximum = maximum;
  }
}

export type ResourceMeasurementErrorCode =
  'invalid_unicode' | 'unsupported_json_value' | 'non_finite_json_number' | 'circular_json_value';

export class ResourceMeasurementError extends TypeError {
  readonly code: ResourceMeasurementErrorCode;

  constructor(code: ResourceMeasurementErrorCode) {
    super(messageForMeasurementError(code));
    this.name = 'ResourceMeasurementError';
    this.code = code;
  }
}

export function trimmedUnicodeScalarCount(value: string): number {
  const scalars = unicodeScalars(value);
  let first = 0;
  while (first < scalars.length) {
    const scalar = scalars[first];
    if (scalar === undefined || !isUnicodeWhitespace(scalar)) break;
    first += 1;
  }

  let last = scalars.length;
  while (last > first) {
    const scalar = scalars[last - 1];
    if (scalar === undefined || !isUnicodeWhitespace(scalar)) break;
    last -= 1;
  }
  return last - first;
}

export function compactJsonUtf8Bytes(value: unknown): number {
  assertJsonValue(value, new WeakSet());
  return utf8ByteLength(JSON.stringify(value));
}

export function byteLength(value: Uint8Array): number {
  return value.byteLength;
}

export function collectionLength(value: readonly unknown[]): number {
  return value.length;
}

export function checkMeasurement(measured: number, allowed: number): LimitMeasurement {
  assertNonNegativeSafeInteger(measured, 'measured');
  assertNonNegativeSafeInteger(allowed, 'allowed');
  if (measured > allowed) throw new LimitExceededError(measured, allowed);
  return { measured, allowed };
}

export function checkRange(measured: number, minimum: number, maximum: number): LimitRange {
  assertNonNegativeSafeInteger(measured, 'measured');
  assertNonNegativeSafeInteger(minimum, 'minimum');
  assertNonNegativeSafeInteger(maximum, 'maximum');
  if (minimum > maximum) throw new RangeError('minimum must not exceed maximum');
  if (measured < minimum || measured > maximum) {
    throw new LimitOutOfRangeError(measured, minimum, maximum);
  }
  return { measured, minimum, maximum };
}

export function checkTrimmedUnicodeScalars(value: string, allowed: number): LimitMeasurement {
  return checkMeasurement(trimmedUnicodeScalarCount(value), allowed);
}

export function checkCompactJsonUtf8Bytes(value: unknown, allowed: number): LimitMeasurement {
  return checkMeasurement(compactJsonUtf8Bytes(value), allowed);
}

export function checkBytes(value: Uint8Array, allowed: number): LimitMeasurement {
  return checkMeasurement(byteLength(value), allowed);
}

export function checkCollection(value: readonly unknown[], allowed: number): LimitMeasurement {
  return checkMeasurement(collectionLength(value), allowed);
}

export interface LimitsPolicySchema {
  readonly version: number;
  readonly sections: Readonly<Record<string, readonly string[]>>;
  readonly allowZero?: readonly string[];
  readonly rangeKeys?: readonly string[];
}

type PolicyValue<Schema extends LimitsPolicySchema, Path extends string> = Schema extends {
  readonly rangeKeys: readonly (infer RangePath)[];
}
  ? Path extends RangePath
    ? LimitBounds
    : number
  : number;

export type LimitsPolicy<Schema extends LimitsPolicySchema> = {
  readonly $comment: string;
  readonly version: number;
} & {
  readonly [Section in keyof Schema['sections']]: {
    readonly [Key in Schema['sections'][Section][number]]: PolicyValue<
      Schema,
      `${Section & string}.${Key}`
    >;
  };
};

export class LimitsPolicyError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'LimitsPolicyError';
  }
}

export class LimitError<Reason extends string = string> extends Error implements LimitMeasurement {
  readonly reason: Reason;
  readonly field: string;
  readonly measured: number;
  readonly allowed: number;

  constructor(reason: Reason, field: string, measurement: LimitMeasurement) {
    super(`Limit exceeded for ${field}: ${reason}`);
    this.name = 'LimitError';
    this.reason = reason;
    this.field = field;
    this.measured = measurement.measured;
    this.allowed = measurement.allowed;
  }
}

export class LimitRangeError extends LimitError<'out_of_range'> implements LimitRange {
  readonly minimum: number;
  readonly maximum: number;

  constructor(field: string, measurement: LimitRange) {
    super('out_of_range', field, { measured: measurement.measured, allowed: measurement.maximum });
    this.name = 'LimitRangeError';
    this.minimum = measurement.minimum;
    this.maximum = measurement.maximum;
  }
}

const POLICY_ROOT = 'limits';
const RESERVED_POLICY_KEYS: readonly string[] = ['$comment', 'version'];

export function parseLimitsPolicy<const Schema extends LimitsPolicySchema>(
  value: unknown,
  schema: Schema,
): LimitsPolicy<Schema> {
  assertPolicySchema(schema);
  const policy = expectPolicyObject(value, POLICY_ROOT);
  expectExactKeys(policy, [...RESERVED_POLICY_KEYS, ...Object.keys(schema.sections)], POLICY_ROOT);
  if (typeof policy['$comment'] !== 'string') {
    throw new LimitsPolicyError(`${POLICY_ROOT}.$comment must be a string`);
  }
  if (policy['version'] !== schema.version) {
    throw new LimitsPolicyError(`Unsupported limits policy version ${String(policy['version'])}`);
  }
  const allowZero = new Set(schema.allowZero ?? []);
  const rangeKeys = new Set(schema.rangeKeys ?? []);
  for (const [section, keys] of Object.entries(schema.sections)) {
    checkPolicySection(policy, section, keys, allowZero, rangeKeys);
  }
  return policy as LimitsPolicy<Schema>;
}

export function enforceLimit<Measurement extends LimitMeasurement | LimitRange>(
  field: string,
  reason: string,
  check: () => Measurement,
): Measurement {
  try {
    return check();
  } catch (error) {
    if (error instanceof LimitExceededError) throw new LimitError(reason, field, error);
    if (error instanceof LimitOutOfRangeError) throw new LimitRangeError(field, error);
    throw error;
  }
}

function assertPolicySchema(schema: LimitsPolicySchema): void {
  const known = new Set<string>();
  for (const [section, keys] of Object.entries(schema.sections)) {
    if (RESERVED_POLICY_KEYS.includes(section)) {
      throw new TypeError(`Limits policy section ${section} is reserved`);
    }
    for (const key of keys) known.add(`${section}.${key}`);
  }
  for (const path of schema.allowZero ?? []) {
    if (!known.has(path)) throw new TypeError(`Limits policy allowZero names unknown ${path}`);
  }
  for (const path of schema.rangeKeys ?? []) {
    if (!known.has(path)) throw new TypeError(`Limits policy rangeKeys names unknown ${path}`);
    if (schema.allowZero?.includes(path)) {
      throw new TypeError(`Limits policy range key ${path} cannot also use allowZero`);
    }
  }
}

function checkPolicySection(
  policy: Record<string, unknown>,
  section: string,
  keys: readonly string[],
  allowZero: ReadonlySet<string>,
  rangeKeys: ReadonlySet<string>,
): void {
  const path = `${POLICY_ROOT}.${section}`;
  const values = expectPolicyObject(policy[section], path);
  expectExactKeys(values, keys, path);
  for (const key of keys) {
    if (rangeKeys.has(`${section}.${key}`)) {
      checkPolicyRange(values[key], `${path}.${key}`);
      continue;
    }
    const minimum = allowZero.has(`${section}.${key}`) ? 0 : 1;
    const limit = values[key];
    if (typeof limit !== 'number' || !Number.isSafeInteger(limit) || limit < minimum) {
      const expected = minimum === 0 ? 'a non-negative integer' : 'a positive integer';
      throw new LimitsPolicyError(`${path}.${key} must be ${expected}`);
    }
  }
}

function checkPolicyRange(value: unknown, path: string): void {
  const bounds = expectPolicyObject(value, path);
  expectExactKeys(bounds, ['minimum', 'maximum'], path);
  for (const key of ['minimum', 'maximum']) {
    const bound = bounds[key];
    if (typeof bound !== 'number' || !Number.isSafeInteger(bound) || bound < 0) {
      throw new LimitsPolicyError(`${path}.${key} must be a non-negative integer`);
    }
  }
  if (Number(bounds['minimum']) > Number(bounds['maximum'])) {
    throw new LimitsPolicyError(`${path}.minimum must not exceed maximum`);
  }
}

function expectPolicyObject(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new LimitsPolicyError(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function expectExactKeys(
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

function unicodeScalars(value: string): number[] {
  const scalars: number[] = [];
  for (let index = 0; index < value.length; index += 1) {
    const first = value.charCodeAt(index);
    if (isHighSurrogate(first)) {
      const second = value.charCodeAt(index + 1);
      if (!isLowSurrogate(second)) throw new ResourceMeasurementError('invalid_unicode');
      scalars.push((first - 0xd800) * 0x400 + second - 0xdc00 + 0x10000);
      index += 1;
      continue;
    }
    if (isLowSurrogate(first)) throw new ResourceMeasurementError('invalid_unicode');
    scalars.push(first);
  }
  return scalars;
}

function isHighSurrogate(value: number): boolean {
  return value >= 0xd800 && value <= 0xdbff;
}

function isLowSurrogate(value: number): boolean {
  return value >= 0xdc00 && value <= 0xdfff;
}

function isUnicodeWhitespace(value: number): boolean {
  return (
    (value >= 0x0009 && value <= 0x000d) ||
    value === 0x0020 ||
    value === 0x0085 ||
    value === 0x00a0 ||
    value === 0x1680 ||
    (value >= 0x2000 && value <= 0x200a) ||
    value === 0x2028 ||
    value === 0x2029 ||
    value === 0x202f ||
    value === 0x205f ||
    value === 0x3000
  );
}

function assertJsonValue(value: unknown, ancestors: WeakSet<object>): void {
  if (value === null || typeof value === 'boolean') return;
  if (typeof value === 'string') {
    unicodeScalars(value);
    return;
  }
  if (typeof value === 'number') {
    if (!Number.isFinite(value)) {
      throw new ResourceMeasurementError('non_finite_json_number');
    }
    return;
  }
  if (typeof value !== 'object') {
    throw new ResourceMeasurementError('unsupported_json_value');
  }
  if (ancestors.has(value)) throw new ResourceMeasurementError('circular_json_value');

  ancestors.add(value);
  try {
    if (Object.getOwnPropertySymbols(value).length > 0) {
      throw new ResourceMeasurementError('unsupported_json_value');
    }
    if (Array.isArray(value)) {
      for (let index = 0; index < value.length; index += 1) {
        const descriptor = Object.getOwnPropertyDescriptor(value, index);
        if (descriptor === undefined || !('value' in descriptor)) {
          throw new ResourceMeasurementError('unsupported_json_value');
        }
        assertJsonValue(descriptor.value, ancestors);
      }
      return;
    }

    const prototype = Object.getPrototypeOf(value) as object | null;
    if (prototype !== Object.prototype && prototype !== null) {
      throw new ResourceMeasurementError('unsupported_json_value');
    }
    for (const key of Object.keys(value)) {
      unicodeScalars(key);
      const descriptor = Object.getOwnPropertyDescriptor(value, key);
      if (descriptor === undefined || !('value' in descriptor)) {
        throw new ResourceMeasurementError('unsupported_json_value');
      }
      assertJsonValue(descriptor.value, ancestors);
    }
  } finally {
    ancestors.delete(value);
  }
}

function utf8ByteLength(value: string): number {
  let length = 0;
  for (const scalar of unicodeScalars(value)) {
    if (scalar <= 0x7f) length += 1;
    else if (scalar <= 0x7ff) length += 2;
    else if (scalar <= 0xffff) length += 3;
    else length += 4;
  }
  return length;
}

function assertNonNegativeSafeInteger(value: number, name: string): void {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new RangeError(`${name} must be a non-negative safe integer`);
  }
}

function messageForMeasurementError(code: ResourceMeasurementErrorCode): string {
  switch (code) {
    case 'invalid_unicode':
      return 'Strings must contain valid Unicode scalar values';
    case 'unsupported_json_value':
      return 'The value is not supported by compact JSON measurement';
    case 'non_finite_json_number':
      return 'Compact JSON measurement requires finite numbers';
    case 'circular_json_value':
      return 'Compact JSON measurement does not support circular values';
  }
}
