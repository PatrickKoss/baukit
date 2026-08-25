export type LocalePreference<TLocale extends string> = 'system' | TLocale;

const MAX_LOCALE_TAG_LENGTH = 128;

interface SupportedLocale<TLocale extends string> {
  readonly canonical: string;
  readonly locale: TLocale;
}

interface LocaleConfiguration<TLocale extends string> {
  readonly fallback: TLocale;
  readonly supported: readonly SupportedLocale<TLocale>[];
}

function canonicalLocale(value: unknown, allowSurroundingWhitespace: boolean): string | null {
  if (typeof value !== 'string' || value.length === 0 || value.length > MAX_LOCALE_TAG_LENGTH) {
    return null;
  }

  const trimmed = value.trim();
  if (
    trimmed.length === 0 ||
    trimmed.length > MAX_LOCALE_TAG_LENGTH ||
    (!allowSurroundingWhitespace && trimmed !== value)
  ) {
    return null;
  }

  try {
    return Intl.getCanonicalLocales(trimmed.replaceAll('_', '-'))[0] ?? null;
  } catch {
    return null;
  }
}

function localeConfiguration<TLocale extends string>(
  supported: readonly TLocale[],
  fallback: TLocale,
): LocaleConfiguration<TLocale> {
  const normalized: SupportedLocale<TLocale>[] = [];

  for (const locale of supported) {
    const canonical = canonicalLocale(locale, false);
    if (canonical !== null && canonical.toLowerCase() !== 'system') {
      normalized.push({ canonical, locale });
    }
  }

  const canonicalFallback = canonicalLocale(fallback, false);
  const supportedFallback = normalized.find(({ canonical }) => canonical === canonicalFallback);
  if (supportedFallback === undefined) {
    throw new RangeError('fallback must be a valid member of supported locales');
  }

  return { fallback: supportedFallback.locale, supported: normalized };
}

function exactSupportedLocale<TLocale extends string>(
  canonical: string,
  supported: readonly SupportedLocale<TLocale>[],
): TLocale | undefined {
  return supported.find((entry) => entry.canonical === canonical)?.locale;
}

function normalizedPreference<TLocale extends string>(
  value: unknown,
  configuration: LocaleConfiguration<TLocale>,
): LocalePreference<TLocale> {
  if (value === 'system') {
    return 'system';
  }

  const canonical = canonicalLocale(value, false);
  return canonical === null
    ? configuration.fallback
    : (exactSupportedLocale(canonical, configuration.supported) ?? configuration.fallback);
}

export function normalizeLocalePreference<TLocale extends string>(options: {
  readonly value: unknown;
  readonly supported: readonly TLocale[];
  readonly fallback: TLocale;
}): LocalePreference<TLocale> {
  const configuration = localeConfiguration(options.supported, options.fallback);
  return normalizedPreference(options.value, configuration);
}

export function resolveLocale<TLocale extends string>(options: {
  readonly preference: unknown;
  readonly deviceLocales: readonly string[];
  readonly supported: readonly TLocale[];
  readonly fallback: TLocale;
}): TLocale {
  const configuration = localeConfiguration(options.supported, options.fallback);
  const preference = normalizedPreference(options.preference, configuration);
  if (preference !== 'system') {
    return preference;
  }

  const deviceLocales = options.deviceLocales
    .map((locale) => canonicalLocale(locale, true))
    .filter((locale): locale is string => locale !== null);

  for (const deviceLocale of deviceLocales) {
    const exact = exactSupportedLocale(deviceLocale, configuration.supported);
    if (exact !== undefined) {
      return exact;
    }
  }

  for (const deviceLocale of deviceLocales) {
    const base = deviceLocale.split('-', 1)[0];
    if (base === undefined) {
      continue;
    }

    const exactBase = exactSupportedLocale(base, configuration.supported);
    if (exactBase !== undefined) {
      return exactBase;
    }

    const regionalBase = configuration.supported.find(
      ({ canonical }) => canonical.split('-', 1)[0] === base,
    );
    if (regionalBase !== undefined) {
      return regionalBase.locale;
    }
  }

  return configuration.fallback;
}

export function canonicalizeResolvedLocale(locale: string): string {
  const canonical = canonicalLocale(locale, false);
  const base = canonical?.split('-', 1)[0]?.toLowerCase();
  if (canonical === null || base === 'system' || base === 'und') {
    throw new RangeError('locale must be an explicit, valid locale tag');
  }
  return canonical;
}
