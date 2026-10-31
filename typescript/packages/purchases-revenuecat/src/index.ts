import {
  isPurchaseError,
  PurchaseError,
  type PurchaseOffering,
  type PurchaseOfferings,
  type PurchasePackage,
  type PurchasesPort,
} from '@baukit/purchases-core';
import type {
  CustomerInfoUpdateListener,
  PURCHASES_ERROR_CODE,
  PurchasesConfiguration,
  PurchasesError,
  PurchasesOffering,
  PurchasesOfferings,
  PurchasesPackage,
} from 'react-native-purchases';

/** The calls this adapter makes on the `react-native-purchases` default export. */
export interface RevenueCatModule {
  readonly PURCHASES_ERROR_CODE: typeof PURCHASES_ERROR_CODE;
  configure(configuration: PurchasesConfiguration): void;
  logIn(appUserID: string): Promise<unknown>;
  logOut(): Promise<unknown>;
  isAnonymous(): Promise<boolean>;
  getOfferings(): Promise<PurchasesOfferings>;
  purchasePackage(aPackage: PurchasesPackage): Promise<unknown>;
  restorePurchases(): Promise<unknown>;
  addCustomerInfoUpdateListener(listener: CustomerInfoUpdateListener): void;
  removeCustomerInfoUpdateListener(listener: CustomerInfoUpdateListener): boolean;
}

export interface RevenueCatPurchasesOptions {
  /** Public RevenueCat SDK key for the running platform, from app configuration. */
  readonly apiKey: string;
}

type ErrorCodes = RevenueCatModule['PURCHASES_ERROR_CODE'];

function packageKey(offeringId: string, packageId: string): string {
  return `${offeringId}\u0000${packageId}`;
}

function translate(codes: ErrorCodes, error: unknown): PurchaseError {
  if (isPurchaseError(error)) return error;
  if (typeof error !== 'object' || error === null)
    return new PurchaseError('store', { cause: error });
  const { code } = error as Partial<PurchasesError>;
  if (code === codes.PURCHASE_CANCELLED_ERROR)
    return new PurchaseError('cancelled', { cause: error });
  if (code === codes.PAYMENT_PENDING_ERROR) return new PurchaseError('pending', { cause: error });
  if (code === codes.NETWORK_ERROR || code === codes.OFFLINE_CONNECTION_ERROR)
    return new PurchaseError('network', { cause: error });
  return new PurchaseError('store', { cause: error });
}

const NON_SUBSCRIPTION = 'NON_SUBSCRIPTION';

function mapPackage(offeringId: string, item: PurchasesPackage): PurchasePackage {
  const category: string | null = item.product.productCategory;
  return {
    id: item.identifier,
    offeringId,
    productId: item.product.identifier,
    kind: category === NON_SUBSCRIPTION ? 'one_time' : 'subscription',
    title: item.product.title,
    description: item.product.description,
    priceLabel: item.product.priceString,
  };
}

class PackageCatalog {
  readonly #packages = new Map<string, PurchasesPackage>();

  replace(offerings: PurchasesOfferings): PurchaseOfferings {
    this.#packages.clear();
    const all: Record<string, PurchaseOffering> = {};
    for (const [id, offering] of Object.entries(offerings.all)) all[id] = this.#offering(offering);
    const current = offerings.current ? (all[offerings.current.identifier] ?? null) : null;
    return { current, all };
  }

  find(item: PurchasePackage): PurchasesPackage {
    const found = this.#packages.get(packageKey(item.offeringId, item.id));
    if (found === undefined) throw new PurchaseError('store');
    return found;
  }

  clear(): void {
    this.#packages.clear();
  }

  #offering(offering: PurchasesOffering): PurchaseOffering {
    const packages = offering.availablePackages.map((item) => {
      this.#packages.set(packageKey(offering.identifier, item.identifier), item);
      return mapPackage(offering.identifier, item);
    });
    return { id: offering.identifier, packages };
  }
}

/** Configures RevenueCat once. Pass the `react-native-purchases` default export. */
export function createRevenueCatPurchasesPort(
  purchases: RevenueCatModule,
  options: RevenueCatPurchasesOptions,
): PurchasesPort {
  if (options.apiKey.trim() === '') throw new PurchaseError('unavailable');
  purchases.configure({ apiKey: options.apiKey });
  const catalog = new PackageCatalog();
  const call = async <T>(operation: () => Promise<T>): Promise<T> => {
    try {
      return await operation();
    } catch (error) {
      throw translate(purchases.PURCHASES_ERROR_CODE, error);
    }
  };

  return {
    identify: (suiteSubject) =>
      call(async () => {
        await purchases.logIn(suiteSubject);
      }),
    reset: () =>
      call(async () => {
        catalog.clear();
        if (await purchases.isAnonymous()) return;
        await purchases.logOut();
      }),
    offerings: () => call(async () => catalog.replace(await purchases.getOfferings())),
    purchase: (item) =>
      call(async () => {
        await purchases.purchasePackage(catalog.find(item));
      }),
    restore: () =>
      call(async () => {
        await purchases.restorePurchases();
      }),
    onCustomerUpdate: (listener) => {
      const forward = () => {
        listener();
      };
      purchases.addCustomerInfoUpdateListener(forward);
      return () => {
        purchases.removeCustomerInfoUpdateListener(forward);
      };
    },
  };
}
