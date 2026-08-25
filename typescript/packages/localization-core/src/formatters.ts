import { canonicalizeResolvedLocale } from './locale.js';

function supportedNumberLocale(locale: string): string {
  const canonical = canonicalizeResolvedLocale(locale);
  if (Intl.NumberFormat.supportedLocalesOf([canonical], { localeMatcher: 'lookup' }).length === 0) {
    throw new RangeError(`number formatting does not support locale ${canonical}`);
  }
  return canonical;
}

function supportedDateTimeLocale(locale: string): string {
  const canonical = canonicalizeResolvedLocale(locale);
  if (
    Intl.DateTimeFormat.supportedLocalesOf([canonical], { localeMatcher: 'lookup' }).length === 0
  ) {
    throw new RangeError(`date and time formatting does not support locale ${canonical}`);
  }
  return canonical;
}

export function createNumberFormatter(
  locale: string,
  options?: Intl.NumberFormatOptions,
): Intl.NumberFormat {
  return new Intl.NumberFormat(supportedNumberLocale(locale), options);
}

export function createDateTimeFormatter(
  locale: string,
  options?: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
  return new Intl.DateTimeFormat(supportedDateTimeLocale(locale), options);
}
