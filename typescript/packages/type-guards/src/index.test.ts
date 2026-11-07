import { describe, expect, it } from 'vitest';
import {
  isBoolean,
  isNonEmptyString,
  isNonNegativeSafeInteger,
  isRecord,
  isSafeInteger,
  isString,
  isStringArray,
  nullable,
} from './index.js';

describe('type guards', () => {
  it('accepts only plain objects as records', () => {
    expect(isRecord({ a: 1 })).toBe(true);
    expect(isRecord(Object.create(null))).toBe(true);
    expect(isRecord([])).toBe(false);
    expect(isRecord(null)).toBe(false);
    expect(isRecord('x')).toBe(false);
  });

  it('separates strings from non-empty strings', () => {
    expect(isString('')).toBe(true);
    expect(isNonEmptyString('')).toBe(false);
    expect(isNonEmptyString('a')).toBe(true);
    expect(isString(1)).toBe(false);
  });

  it('accepts safe integers and rejects fractions, NaN and huge values', () => {
    expect(isSafeInteger(-3)).toBe(true);
    expect(isSafeInteger(1.5)).toBe(false);
    expect(isSafeInteger(Number.NaN)).toBe(false);
    expect(isSafeInteger(2 ** 53)).toBe(false);
    expect(isNonNegativeSafeInteger(0)).toBe(true);
    expect(isNonNegativeSafeInteger(-1)).toBe(false);
  });

  it('checks booleans and string arrays', () => {
    expect(isBoolean(false)).toBe(true);
    expect(isBoolean(0)).toBe(false);
    expect(isStringArray(['a', 'b'])).toBe(true);
    expect(isStringArray(['a', 1])).toBe(false);
    expect(isStringArray('a')).toBe(false);
  });

  it('extends a guard with null', () => {
    const nullableString = nullable(isString);
    expect(nullableString(null)).toBe(true);
    expect(nullableString('a')).toBe(true);
    expect(nullableString(undefined)).toBe(false);
  });
});
