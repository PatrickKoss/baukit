import { describe, expect, it } from 'vitest';

import { defineCatalogSegment, type LocalizedCatalogSegment } from './index.js';

const english = {
  title: 'Account',
  devices: { one: '{{count}} device', other: '{{count}} devices' },
} as const;

describe('typed catalog segments', () => {
  it('uses any caller-selected reference locale and freezes the locale map', () => {
    const catalogs = defineCatalogSegment(['de', 'en', 'es'] as const, 'en', english, {
      de: { title: 'Konto', devices: { one: '{{count}} Gerät', other: '{{count}} Geräte' } },
      es: {
        title: 'Cuenta',
        devices: { one: '{{count}} dispositivo', other: '{{count}} dispositivos' },
      },
    });

    expect(catalogs.en).toBe(english);
    expect(catalogs.de.devices.other).toBe('{{count}} Geräte');
    expect(Object.isFrozen(catalogs)).toBe(true);
  });

  it('rejects invalid dynamic locale lists', () => {
    expect(() =>
      defineCatalogSegment(['en', 'en'] as [string, ...string[]], 'en', english, {}),
    ).toThrow('Supported locales must not contain duplicates.');
    expect(() =>
      defineCatalogSegment(['en', 'de'] as [string, ...string[]], 'en', english, {}),
    ).toThrow('Localized catalogs must match the supported locales.');
  });

  it('keeps exact message keys and string versus plural shape in the public type', () => {
    const valid: LocalizedCatalogSegment<typeof english> = {
      title: 'Konto',
      devices: { one: 'Gerät', other: 'Geräte' },
    };
    expect(valid.title).toBe('Konto');

    const missingKey: LocalizedCatalogSegment<typeof english> = {
      // @ts-expect-error The reference locale requires devices.
      title: 'Konto',
    };
    const wrongShape: LocalizedCatalogSegment<typeof english> = {
      title: 'Konto',
      // @ts-expect-error A plural reference message requires one and other.
      devices: 'Geräte',
    };
    const extraKey: LocalizedCatalogSegment<typeof english> = {
      title: 'Konto',
      devices: { one: 'Gerät', other: 'Geräte' },
      // @ts-expect-error Translations cannot add keys absent from the reference.
      retry: 'Erneut versuchen',
    };
    expect(missingKey.title).toBe('Konto');
    expect(wrongShape.devices).toBe('Geräte');
    expect(extraKey.title).toBe('Konto');
  });
});
