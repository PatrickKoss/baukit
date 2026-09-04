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
