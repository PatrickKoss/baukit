import { readFileSync } from 'node:fs';

import { describe, expect, it, vi } from 'vitest';

import {
  catalogKeySet,
  compareCatalogKeys,
  createDateTimeFormatter,
  createLocalizedCodeResolver,
  createNumberFormatter,
  normalizeLocalePreference,
  resolveLocale,
} from './index.js';

interface LocaleResolutionCase {
  readonly name: string;
  readonly preference: unknown;
  readonly deviceLocales: readonly string[];
  readonly supported: readonly string[];
  readonly fallback: string;
  readonly expected: string;
}

const fixtureUrl = new URL(
  '../../../../fixtures/product-experience/locale-resolution.json',
  import.meta.url,
);
const readUtf8File = readFileSync as unknown as (path: URL, encoding: 'utf8') => string;
const localeCases = JSON.parse(readUtf8File(fixtureUrl, 'utf8')) as readonly LocaleResolutionCase[];

describe('locale preference normalization', () => {
  const options = { supported: ['en', 'de', 'de-AT'] as const, fallback: 'en' as const };

  it('keeps system and returns the allowlisted spelling for normalized locale tags', () => {
    expect(normalizeLocalePreference({ ...options, value: 'system' })).toBe('system');
    expect(normalizeLocalePreference({ ...options, value: 'DE_at' })).toBe('de-AT');
  });

  it.each([
    undefined,
    null,
    false,
    12,
    [],
    {},
    new String('de'),
    Symbol('de'),
    '__proto__',
    'constructor',
    'de/../../catalog',
    '',
    '   ',
    ' de ',
    'a'.repeat(129),
  ])('normalizes malformed value %p to the fallback', (value) => {
    expect(normalizeLocalePreference({ ...options, value })).toBe('en');
  });

  it('rejects a fallback outside the valid supported locale set', () => {
    expect(() =>
      normalizeLocalePreference({ value: 'system', supported: ['de'] as const, fallback: 'en' }),
    ).toThrow(RangeError);
  });
});

describe('locale resolution fixtures', () => {
  it.each(localeCases)('$name', ({ preference, deviceLocales, supported, fallback, expected }) => {
    expect(resolveLocale({ preference, deviceLocales, supported, fallback })).toBe(expected);
  });

  it('executes every shared fixture case', () => {
    expect(localeCases).toHaveLength(18);
  });
});

describe('catalog key comparison', () => {
  it('returns the sorted recursive leaf-key set', () => {
    expect(
      catalogKeySet({
        title: 'Title',
        account: { actions: { delete: 'Delete' }, empty: {} },
        choices: ['one', 'two'],
      }),
    ).toEqual(['account.actions.delete', 'account.empty', 'choices', 'title']);
  });

  it('reports exact missing and extra keys without comparing translated values', () => {
    const reference = { actions: { save: 'Save', cancel: 'Cancel' }, title: 'Settings' };
    const candidate = { actions: { save: 'Speichern', retry: 'Erneut versuchen' }, title: '' };

    expect(compareCatalogKeys(reference, candidate)).toEqual({
      missing: ['actions.cancel'],
      extra: ['actions.retry'],
    });
  });

  it('supports exact immutable catalog-ID coverage', () => {
    const emittedIds = { account_locked: true, rate_limited: true };
    const translatedIds = { account_locked: 'Konto gesperrt', stale_code: 'Veraltet' };
    expect(compareCatalogKeys(emittedIds, translatedIds)).toEqual({
      missing: ['rate_limited'],
      extra: ['stale_code'],
    });
  });

  it('does not confuse dotted keys with nested paths', () => {
    expect(catalogKeySet({ 'account.title': 'literal', account: { title: 'nested' } })).toEqual([
      'account.title',
      'account\\.title',
    ]);
  });

  it('handles malformed and cyclic catalogs without invoking getters', () => {
    const catalog: Record<string, unknown> = { safe: 'text' };
    catalog['cycle'] = catalog;
    Object.defineProperty(catalog, 'getter', {
      enumerable: true,
      get: () => {
        throw new Error('must not run');
      },
    });

    expect(catalogKeySet(catalog)).toEqual(['cycle', 'getter', 'safe']);
    expect(catalogKeySet(null)).toEqual([]);
    expect(catalogKeySet(['not', 'a', 'catalog'])).toEqual([]);
  });
});

describe('localized backend codes', () => {
  type Code = 'invalid_name' | 'rate_limited' | 'unknown_code';

  it('resolves stable codes and passes structured details to message renderers', () => {
    const fallback = vi.fn(() => 'Safe API message');
    const resolve = createLocalizedCodeResolver<Code>({
      catalog: {
        invalid_name: 'Enter a name.',
        rate_limited: (details) => {
          if (
            details === null ||
            typeof details !== 'object' ||
            !('retryAfter' in details) ||
            typeof details.retryAfter !== 'number'
          ) {
            return undefined;
          }
          return `Try again in ${String(details.retryAfter)} seconds.`;
        },
      },
      fallback,
    });

    expect(resolve('invalid_name')).toBe('Enter a name.');
    expect(resolve('rate_limited', { retryAfter: 30 })).toBe('Try again in 30 seconds.');
    expect(fallback).not.toHaveBeenCalled();
  });

  it('uses the safe API message for unknown, empty, or unrenderable entries', () => {
    const apiMessage = 'The request could not be completed.';
    const resolve = createLocalizedCodeResolver<Code>({
      catalog: {
        invalid_name: '   ',
        rate_limited: () => {
          throw new Error('bad details');
        },
      },
      fallback: apiMessage,
    });

    expect(resolve('unknown_code')).toBe(apiMessage);
    expect(resolve('invalid_name')).toBe(apiMessage);
    expect(resolve('rate_limited', {})).toBe(apiMessage);
  });

  it('does not resolve object-prototype properties as catalog codes', () => {
    const resolve = createLocalizedCodeResolver({
      catalog: {},
      fallback: 'Safe API message',
    });
    expect(resolve('__proto__')).toBe('Safe API message');
  });
});

describe('explicit-locale formatters', () => {
  it('creates number and unit formatters with the supplied resolved locale', () => {
    const english = createNumberFormatter('en-US', { style: 'unit', unit: 'kilometer' });
    const german = createNumberFormatter('de_DE', { style: 'unit', unit: 'kilometer' });

    expect(english.resolvedOptions().locale).toBe('en-US');
    expect(german.resolvedOptions().locale).toBe('de-DE');
    expect(english.format(1234.5)).not.toBe(german.format(1234.5));
  });

  it('creates date and time formatters with the supplied locale and options', () => {
    const british = createDateTimeFormatter('en-GB', {
      dateStyle: 'short',
      timeStyle: 'short',
      timeZone: 'UTC',
    });
    const german = createDateTimeFormatter('de-DE', {
      dateStyle: 'short',
      timeStyle: 'short',
      timeZone: 'UTC',
    });
    const instant = new Date('2024-01-02T15:04:00Z');

    expect(british.resolvedOptions().locale).toBe('en-GB');
    expect(german.resolvedOptions().locale).toBe('de-DE');
    expect(british.format(instant)).not.toBe(german.format(instant));
  });

  it('rejects missing, blank, padded, and malformed locale values', () => {
    expect(() => createNumberFormatter(undefined as never)).toThrow(RangeError);
    expect(() => createNumberFormatter('')).toThrow(RangeError);
    expect(() => createNumberFormatter(' de ')).toThrow(RangeError);
    expect(() => createNumberFormatter('system')).toThrow(RangeError);
    expect(() => createNumberFormatter('und')).toThrow(RangeError);
    expect(() => createNumberFormatter('zz')).toThrow(RangeError);
    expect(() => createDateTimeFormatter('../de')).toThrow(RangeError);
  });
});
