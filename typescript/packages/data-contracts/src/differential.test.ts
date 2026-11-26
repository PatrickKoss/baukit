import { describe, expect, it } from 'vitest';

import csvVectors from '../../../../fixtures/export-csv/generated-v1.json' with { type: 'json' };
import limitsVectors from '../../../../fixtures/limits/generated-v1.json' with { type: 'json' };

import { CsvEncodeError, encodeCsv } from './export.js';
import { compactJsonUtf8Bytes, trimmedUnicodeScalarCount } from './limits.js';

interface CsvCase {
  readonly cells: readonly (string | number)[];
  readonly expected: { readonly csv?: string; readonly error?: string };
}

interface LimitsCase {
  readonly utf16: readonly number[];
  readonly expected: {
    readonly trimmedUnicodeScalarCount?: number;
    readonly compactJsonUtf8Bytes?: number;
    readonly error?: string;
  };
}

function encodeOutcome(cells: readonly (string | number)[]): CsvCase['expected'] {
  try {
    return { csv: encodeCsv([cells]) };
  } catch (cause) {
    if (cause instanceof CsvEncodeError) return { error: cause.code };
    throw cause;
  }
}

function measureOutcome(units: readonly number[]): LimitsCase['expected'] {
  const text = String.fromCharCode(...units);
  try {
    return {
      trimmedUnicodeScalarCount: trimmedUnicodeScalarCount(text),
      compactJsonUtf8Bytes: compactJsonUtf8Bytes({ value: text, list: [text, 1, -2] }),
    };
  } catch {
    return { error: 'invalid_unicode' };
  }
}

describe('D-06 CSV differential against Rust encode_csv', () => {
  it.each(
    (csvVectors.cases as readonly CsvCase[]).map((vector, index) => [index, vector] as const),
  )('case %i matches', (_index, vector) => {
    expect(encodeOutcome(vector.cells)).toEqual(vector.expected);
  });
});

describe('D-07 limits differential against Rust limits.rs', () => {
  it.each(
    (limitsVectors.cases as readonly LimitsCase[]).map((vector, index) => [index, vector] as const),
  )('case %i matches', (_index, vector) => {
    expect(measureOutcome(vector.utf16)).toEqual(vector.expected);
  });
});
