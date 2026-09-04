export interface PluralMessage {
  readonly one: string;
  readonly other: string;
}

export type CatalogMessage = string | PluralMessage;
export type CatalogSegment = Readonly<Record<string, CatalogMessage>>;

export type LocalizedCatalogSegment<TCatalog extends CatalogSegment> = {
  readonly [TKey in keyof TCatalog]: TCatalog[TKey] extends string
    ? string
    : TCatalog[TKey] extends PluralMessage
      ? PluralMessage
      : never;
};

export type CatalogSegmentLocales<
  TLocales extends string,
  TReferenceLocale extends TLocales,
  TReferenceCatalog extends CatalogSegment,
> = Readonly<
  Record<TReferenceLocale, TReferenceCatalog> &
    Record<Exclude<TLocales, TReferenceLocale>, LocalizedCatalogSegment<TReferenceCatalog>>
>;

/**
 * Defines one product-owned catalog segment against a reference locale.
 *
 * TypeScript rejects missing or extra locale keys, missing or extra message
 * keys, and translations that change a string into a plural message or back.
 */
export function defineCatalogSegment<
  const TLocales extends readonly [string, ...string[]],
  const TReferenceLocale extends TLocales[number],
  const TReferenceCatalog extends CatalogSegment,
>(
  supportedLocales: TLocales,
  referenceLocale: TReferenceLocale,
  referenceCatalog: TReferenceCatalog,
  localizedCatalogs: Readonly<
    Record<Exclude<TLocales[number], TReferenceLocale>, LocalizedCatalogSegment<TReferenceCatalog>>
  >,
): CatalogSegmentLocales<TLocales[number], TReferenceLocale, TReferenceCatalog> {
  if (new Set(supportedLocales).size !== supportedLocales.length) {
    throw new RangeError('Supported locales must not contain duplicates.');
  }
  if (!supportedLocales.includes(referenceLocale)) {
    throw new RangeError('The reference locale must be supported.');
  }

  const expectedLocalizedLocales = supportedLocales.filter((locale) => locale !== referenceLocale);
  const suppliedLocalizedLocales = Object.keys(localizedCatalogs);
  if (
    suppliedLocalizedLocales.length !== expectedLocalizedLocales.length ||
    expectedLocalizedLocales.some(
      (locale) => !Object.prototype.hasOwnProperty.call(localizedCatalogs, locale),
    )
  ) {
    throw new RangeError('Localized catalogs must match the supported locales.');
  }

  return Object.freeze({
    ...localizedCatalogs,
    [referenceLocale]: referenceCatalog,
  }) as CatalogSegmentLocales<TLocales[number], TReferenceLocale, TReferenceCatalog>;
}
