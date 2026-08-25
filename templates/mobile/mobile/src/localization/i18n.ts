import { normalizeLocalePreference, resolveLocale } from '@baukit/localization-core';
import type { LocalePreference as CoreLocalePreference } from '@baukit/localization-core';
import { getLocales } from 'expo-localization';
import { createInstance } from 'i18next';
import { initReactI18next } from 'react-i18next';

import { germanCatalog } from './de';
import { englishCatalog } from './en';

export const supportedLocales = ['en', 'de'] as const;
export type SupportedLocale = (typeof supportedLocales)[number];
export type LocalePreference = CoreLocalePreference<SupportedLocale>;

export const i18n = createInstance();
let initialization: Promise<void> | undefined;

export async function initializeI18n(rawPreference: unknown = 'system'): Promise<void> {
  const preference: LocalePreference = normalizeLocalePreference({
    value: rawPreference,
    supported: supportedLocales,
    fallback: 'en',
  });
  const locale = resolveLocale({
    preference,
    deviceLocales: getLocales().map(({ languageTag }) => languageTag),
    supported: supportedLocales,
    fallback: 'en',
  });

  initialization ??= i18n
    .use(initReactI18next)
    .init({
      defaultNS: 'home',
      fallbackLng: 'en',
      initAsync: false,
      interpolation: { escapeValue: false },
      lng: locale,
      ns: ['bootstrap', 'home'],
      react: { useSuspense: false },
      resources: {
        de: germanCatalog,
        en: englishCatalog,
      },
      supportedLngs: supportedLocales,
    })
    .then(() => undefined);
  await initialization;
  if (i18n.resolvedLanguage !== locale) {
    await i18n.changeLanguage(locale);
  }
}
