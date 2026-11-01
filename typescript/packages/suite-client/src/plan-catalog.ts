export interface FeatureKeyDrift {
  /** Declared by the product but granted by no catalog plan. */
  notInCatalog: string[];
  /** Granted by a catalog plan but not declared by the product. */
  notDeclared: string[];
}

export class PlanCatalogError extends Error {
  override readonly name = 'PlanCatalogError';
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function stringList(value: unknown): string[] {
  if (!Array.isArray(value) || !value.every((item) => typeof item === 'string')) {
    throw new PlanCatalogError('plan catalog feature list is not a string array');
  }
  return value;
}

function includedProducts(catalog: unknown): Record<string, unknown>[] {
  if (!isRecord(catalog) || !Array.isArray(catalog['plans'])) {
    throw new PlanCatalogError('plan catalog has no plans');
  }
  return catalog['plans'].flatMap((plan: unknown) => {
    if (!isRecord(plan) || !Array.isArray(plan['includedProducts'])) {
      throw new PlanCatalogError('plan catalog plan has no includedProducts');
    }
    return plan['includedProducts'].filter(isRecord);
  });
}

/** Every feature key any plan in the parsed hub catalog grants to `productId`. */
export function catalogFeatureKeys(catalog: unknown, productId: string): string[] {
  const keys = new Set<string>();
  for (const included of includedProducts(catalog)) {
    if (included['productId'] !== productId) continue;
    for (const key of stringList(included['features'])) keys.add(key);
  }
  if (keys.size === 0) {
    throw new PlanCatalogError(`plan catalog grants no features to ${productId}`);
  }
  return [...keys].sort();
}

export function featureKeyDrift(
  catalog: unknown,
  productId: string,
  declared: Iterable<string>,
): FeatureKeyDrift {
  const granted = new Set(catalogFeatureKeys(catalog, productId));
  const own = new Set(declared);
  return {
    notInCatalog: [...own].filter((key) => !granted.has(key)).sort(),
    notDeclared: [...granted].filter((key) => !own.has(key)).sort(),
  };
}

/** Contract-test assertion: throws with both differences when they disagree. */
export function assertFeatureKeysMatch(
  catalog: unknown,
  productId: string,
  declared: Iterable<string>,
): void {
  const drift = featureKeyDrift(catalog, productId, declared);
  if (drift.notInCatalog.length === 0 && drift.notDeclared.length === 0) return;
  throw new PlanCatalogError(
    `${productId} feature keys drift from the hub catalog: not in catalog ${JSON.stringify(drift.notInCatalog)}, not declared ${JSON.stringify(drift.notDeclared)}`,
  );
}
