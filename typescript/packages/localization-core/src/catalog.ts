export interface CatalogDifference {
  readonly missing: readonly string[];
  readonly extra: readonly string[];
}

function escapeKeySegment(segment: string): string {
  return segment.replaceAll('\\', '\\\\').replaceAll('.', '\\.');
}

function enumerableProperties(value: object): readonly [string, unknown][] | null {
  try {
    return Object.entries(Object.getOwnPropertyDescriptors(value))
      .filter(([, descriptor]) => descriptor.enumerable)
      .map(([key, descriptor]) => [key, 'value' in descriptor ? descriptor.value : undefined]);
  } catch {
    return null;
  }
}

function plainRecordProperties(value: unknown): readonly [string, unknown][] | null {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    return null;
  }

  try {
    const prototype = Object.getPrototypeOf(value) as unknown;
    if (prototype !== Object.prototype && prototype !== null) {
      return null;
    }
  } catch {
    return null;
  }

  return enumerableProperties(value);
}

function collectCatalogKeys(
  value: unknown,
  prefix: string,
  ancestors: WeakSet<object>,
): readonly string[] {
  const properties = plainRecordProperties(value);
  if (properties === null || properties.length === 0) {
    return prefix === '' ? [] : [prefix];
  }

  if (typeof value !== 'object' || value === null || ancestors.has(value)) {
    return prefix === '' ? [] : [prefix];
  }

  ancestors.add(value);
  const keys = properties.flatMap(([key, child]) => {
    const escapedKey = escapeKeySegment(key);
    const path = prefix === '' ? escapedKey : `${prefix}.${escapedKey}`;
    return collectCatalogKeys(child, path, ancestors);
  });
  ancestors.delete(value);
  return keys;
}

export function catalogKeySet(catalog: unknown): readonly string[] {
  return [...collectCatalogKeys(catalog, '', new WeakSet())].sort();
}

export function compareCatalogKeys(reference: unknown, candidate: unknown): CatalogDifference {
  const referenceKeys = new Set(catalogKeySet(reference));
  const candidateKeys = new Set(catalogKeySet(candidate));

  return {
    missing: [...referenceKeys].filter((key) => !candidateKeys.has(key)).sort(),
    extra: [...candidateKeys].filter((key) => !referenceKeys.has(key)).sort(),
  };
}
