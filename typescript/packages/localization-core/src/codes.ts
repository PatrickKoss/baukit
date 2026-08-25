export type LocalizedCodeEntry = string | ((details?: unknown) => string | null | undefined);

export interface CodeResolverOptions<TCode extends string> {
  readonly catalog: Readonly<Partial<Record<TCode, LocalizedCodeEntry>>>;
  readonly fallback: string | ((code: TCode, details?: unknown) => string);
}

function catalogEntry<TCode extends string>(
  catalog: Readonly<Partial<Record<TCode, LocalizedCodeEntry>>>,
  code: TCode,
): LocalizedCodeEntry | undefined {
  try {
    return Object.hasOwn(catalog, code) ? catalog[code] : undefined;
  } catch {
    return undefined;
  }
}

export function createLocalizedCodeResolver<TCode extends string>(
  options: CodeResolverOptions<TCode>,
): (code: TCode, details?: unknown) => string {
  return (code, details) => {
    const entry = catalogEntry(options.catalog, code);
    try {
      const localized = typeof entry === 'function' ? entry(details) : entry;
      if (typeof localized === 'string' && localized.trim().length > 0) {
        return localized;
      }
    } catch {
      // The safe API message is used when structured details cannot be rendered.
    }

    return typeof options.fallback === 'function'
      ? options.fallback(code, details)
      : options.fallback;
  };
}
