import { describe, expect, it } from 'vitest';

import fixtureCorpus from '../../../../fixtures/limits/resource-budget-measurements-v1.json';

import {
  LimitExceededError,
  ResourceMeasurementError,
  byteLength,
  checkBytes,
  checkCollection,
  checkCompactJsonUtf8Bytes,
  checkMeasurement,
  checkTrimmedUnicodeScalars,
  collectionLength,
  compactJsonUtf8Bytes,
  trimmedUnicodeScalarCount,
} from './limits.js';

interface FixtureCorpus {
  readonly version: number;
  readonly text: readonly {
    readonly name: string;
    readonly value: string;
    readonly trimmed_unicode_scalars: number;
  }[];
  readonly json: readonly {
    readonly name: string;
    readonly value: unknown;
    readonly compact: string;
    readonly utf8_bytes: number;
  }[];
  readonly bytes: readonly {
    readonly name: string;
    readonly value: readonly number[];
    readonly bytes: number;
  }[];
  readonly collections: readonly {
    readonly name: string;
    readonly value: readonly unknown[];
    readonly elements: number;
  }[];
}

const fixtures = fixtureCorpus as FixtureCorpus;

describe('resource-budget measurements', () => {
  it('counts ASCII, composed, decomposed, and joined emoji scalars', () => {
    expect(trimmedUnicodeScalarCount(' plain ')).toBe(5);
    expect(trimmedUnicodeScalarCount('\u00e9')).toBe(1);
    expect(trimmedUnicodeScalarCount('e\u0301')).toBe(2);
    expect(trimmedUnicodeScalarCount('👩‍👩‍👧‍👦')).toBe(7);
  });

  it('rejects unpaired JavaScript surrogates', () => {
    expectMeasurementError(() => trimmedUnicodeScalarCount('\ud800'), 'invalid_unicode');
    expectMeasurementError(() => trimmedUnicodeScalarCount('\udc00'), 'invalid_unicode');
    expectMeasurementError(() => compactJsonUtf8Bytes({ value: '\ud800' }), 'invalid_unicode');
  });

  it('measures compact objects, arrays, escapes, and multibyte strings', () => {
    for (const fixture of fixtures.json) {
      expect(compactJsonUtf8Bytes(fixture.value), fixture.name).toBe(fixture.utf8_bytes);
      expect(JSON.stringify(fixture.value), fixture.name).toBe(fixture.compact);
    }
  });

  it('rejects non-finite and unsupported JavaScript values', () => {
    expectMeasurementError(() => compactJsonUtf8Bytes(Number.NaN), 'non_finite_json_number');
    expectMeasurementError(
      () => compactJsonUtf8Bytes({ value: Number.POSITIVE_INFINITY }),
      'non_finite_json_number',
    );
    expectMeasurementError(() => compactJsonUtf8Bytes(undefined), 'unsupported_json_value');
    expectMeasurementError(() => compactJsonUtf8Bytes(1n), 'unsupported_json_value');
    expectMeasurementError(() => compactJsonUtf8Bytes(() => 1), 'unsupported_json_value');
    expectMeasurementError(
      () => compactJsonUtf8Bytes({ value: undefined }),
      'unsupported_json_value',
    );
    const sparse = new Array<unknown>(2);
    sparse[1] = 1;
    expectMeasurementError(() => compactJsonUtf8Bytes(sparse), 'unsupported_json_value');
    expectMeasurementError(() => compactJsonUtf8Bytes(new Date(0)), 'unsupported_json_value');

    const circular: { self?: unknown } = {};
    circular.self = circular;
    expectMeasurementError(() => compactJsonUtf8Bytes(circular), 'circular_json_value');
  });

  it('uses enumerable own string keys and rejects other own keys', () => {
    const value: Record<string, unknown> = { visible: 1 };
    Object.defineProperty(value, 'hidden', { value: 'private', enumerable: false });
    expect(compactJsonUtf8Bytes(value)).toBe(13);

    const nullPrototype = Object.assign(Object.create(null) as Record<string, unknown>, {
      visible: 1,
    });
    expect(compactJsonUtf8Bytes(nullPrototype)).toBe(13);

    const symbolKeyed = { visible: 1, [Symbol('hidden')]: 2 };
    expectMeasurementError(() => compactJsonUtf8Bytes(symbolKeyed), 'unsupported_json_value');

    const accessor = Object.defineProperty({}, 'value', {
      enumerable: true,
      get: () => 'private',
    });
    expectMeasurementError(() => compactJsonUtf8Bytes(accessor), 'unsupported_json_value');

    const invalidKey = { ['\ud800']: 1 };
    expectMeasurementError(() => compactJsonUtf8Bytes(invalidKey), 'invalid_unicode');
  });

  it('reports byte and collection boundaries', () => {
    const bytes = Uint8Array.from([0, 1, 2]);
    const collection = ['a', 'b', 'c'] as const;
    expect(byteLength(bytes)).toBe(3);
    expect(collectionLength(collection)).toBe(3);
    expect(checkBytes(bytes, 3)).toEqual({ measured: 3, allowed: 3 });
    expect(checkCollection(collection, 3)).toEqual({ measured: 3, allowed: 3 });
    expectLimitExceeded(() => checkBytes(bytes, 2), 3, 2);
    expectLimitExceeded(() => checkCollection(collection, 2), 3, 2);
  });

  it('returns measured and allowed values from every check', () => {
    expect(checkMeasurement(2, 3)).toEqual({ measured: 2, allowed: 3 });
    expect(checkTrimmedUnicodeScalars(' e\u0301 ', 2)).toEqual({ measured: 2, allowed: 2 });
    expect(checkCompactJsonUtf8Bytes({ value: 'é' }, 14)).toEqual({ measured: 14, allowed: 14 });
    expectLimitExceeded(() => checkMeasurement(4, 3), 4, 3);
  });

  it('matches the shared fixture corpus', () => {
    expect(fixtures.version).toBe(1);
    for (const fixture of fixtures.text) {
      expect(trimmedUnicodeScalarCount(fixture.value), fixture.name).toBe(
        fixture.trimmed_unicode_scalars,
      );
    }
    for (const fixture of fixtures.bytes) {
      expect(byteLength(Uint8Array.from(fixture.value)), fixture.name).toBe(fixture.bytes);
    }
    for (const fixture of fixtures.collections) {
      expect(collectionLength(fixture.value), fixture.name).toBe(fixture.elements);
    }
  });
});

function expectMeasurementError(
  operation: () => unknown,
  code: ResourceMeasurementError['code'],
): void {
  try {
    operation();
  } catch (error) {
    expect(error).toBeInstanceOf(ResourceMeasurementError);
    expect((error as ResourceMeasurementError).code).toBe(code);
    return;
  }
  throw new Error(`Expected ResourceMeasurementError with code ${code}`);
}

function expectLimitExceeded(operation: () => unknown, measured: number, allowed: number): void {
  try {
    operation();
  } catch (error) {
    expect(error).toBeInstanceOf(LimitExceededError);
    expect(error).toMatchObject({ measured, allowed });
    return;
  }
  throw new Error('Expected LimitExceededError');
}
