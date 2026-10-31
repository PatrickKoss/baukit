export type PurchaseProductKind = 'subscription' | 'one_time';

export interface PurchasePackage {
  readonly id: string;
  readonly offeringId: string;
  readonly productId: string;
  readonly kind: PurchaseProductKind;
  readonly title: string;
  readonly description: string;
  readonly priceLabel: string;
}

export interface PurchaseOffering {
  readonly id: string;
  readonly packages: readonly PurchasePackage[];
}

export interface PurchaseOfferings {
  readonly current: PurchaseOffering | null;
  readonly all: Readonly<Record<string, PurchaseOffering>>;
}

/** Store boundary. Implementations throw `PurchaseError` with a store-neutral code. */
export interface PurchasesPort {
  identify(suiteSubject: string): Promise<void>;
  reset(): Promise<void>;
  offerings(): Promise<PurchaseOfferings>;
  purchase(item: PurchasePackage): Promise<void>;
  restore(): Promise<void>;
  onCustomerUpdate(listener: () => void): () => void;
}

/** Returns the configured offering when it exists, otherwise the store's current offering. */
export function selectOffering(
  offerings: PurchaseOfferings,
  configuredId?: string,
): PurchaseOffering | null {
  if (configuredId) return offerings.all[configuredId] ?? null;
  return offerings.current;
}
