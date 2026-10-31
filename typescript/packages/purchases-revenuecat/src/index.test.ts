import { PurchaseFlow, type PurchaseOffering, type PurchasePackage } from '@baukit/purchases-core';
import type {
  PurchasesOfferings,
  PurchasesPackage,
  PURCHASES_ERROR_CODE as ErrorCodeEnum,
} from 'react-native-purchases';
import { describe, expect, it, vi } from 'vitest';

import { createRevenueCatPurchasesPort, type RevenueCatModule } from './index.js';

const SUBJECT = 'suite|person-1';
const API_KEY = 'appl_public_key';

const CODES = {
  PURCHASE_CANCELLED_ERROR: '1',
  STORE_PROBLEM_ERROR: '2',
  NETWORK_ERROR: '10',
  PAYMENT_PENDING_ERROR: '20',
  OFFLINE_CONNECTION_ERROR: '35',
} as unknown as typeof ErrorCodeEnum;

function nativePackage(
  offering: string,
  id: string,
  product: string,
  category: string | null,
): PurchasesPackage {
  return {
    identifier: id,
    offeringIdentifier: offering,
    presentedOfferingContext: { offeringIdentifier: offering },
    product: {
      identifier: product,
      title: `${product} title`,
      description: `${product} description`,
      priceString: '€4.99',
      productCategory: category,
    },
  } as unknown as PurchasesPackage;
}

const monthly = nativePackage('default', '$rc_monthly', 'app.premium.monthly', 'SUBSCRIPTION');
const small = nativePackage('packs', 'small', 'app.pack.small', 'NON_SUBSCRIPTION');
const OFFERINGS = {
  current: { identifier: 'default', availablePackages: [monthly] },
  all: {
    default: { identifier: 'default', availablePackages: [monthly] },
    packs: { identifier: 'packs', availablePackages: [small] },
  },
} as unknown as PurchasesOfferings;

function firstPackage(offerings: { readonly current: PurchaseOffering | null }): PurchasePackage {
  const item = offerings.current?.packages[0];
  if (item === undefined) throw new Error('offering has no package');
  return item;
}

function mockPurchases() {
  const calls: string[] = [];
  const listeners = new Set<() => void>();
  let anonymous = true;
  const record =
    <T>(name: string, result: () => T) =>
    (...args: unknown[]) => {
      calls.push(args.length ? `${name}:${JSON.stringify(args[0])}` : name);
      return result();
    };
  const module = {
    PURCHASES_ERROR_CODE: CODES,
    configure: vi.fn(record('configure', () => undefined)),
    logIn: vi.fn(
      record('logIn', () => {
        anonymous = false;
        return Promise.resolve({ customerInfo: {}, created: false });
      }),
    ),
    logOut: vi.fn(
      record('logOut', () => {
        anonymous = true;
        return Promise.resolve({});
      }),
    ),
    isAnonymous: vi.fn(() => Promise.resolve(anonymous)),
    getOfferings: vi.fn(record('getOfferings', () => Promise.resolve(OFFERINGS))),
    purchasePackage: vi.fn((item: PurchasesPackage) => {
      calls.push(`purchasePackage:${item.identifier}`);
      return Promise.resolve({});
    }),
    restorePurchases: vi.fn(record('restorePurchases', () => Promise.resolve({}))),
    addCustomerInfoUpdateListener: vi.fn((listener: () => void) => {
      listeners.add(listener);
    }),
    removeCustomerInfoUpdateListener: vi.fn((listener: () => void) => listeners.delete(listener)),
  };
  return {
    module: module as unknown as RevenueCatModule,
    raw: module,
    calls,
    emit: () => {
      for (const listener of listeners) listener();
    },
  };
}

describe('createRevenueCatPurchasesPort', () => {
  it('configures once with the platform key and no app user ID', () => {
    const { module, raw } = mockPurchases();

    createRevenueCatPurchasesPort(module, { apiKey: API_KEY });

    expect(raw.configure).toHaveBeenCalledTimes(1);
    expect(raw.configure).toHaveBeenCalledWith({ apiKey: API_KEY });
  });

  it('refuses an empty key', () => {
    const { module, raw } = mockPurchases();

    expect(() => createRevenueCatPurchasesPort(module, { apiKey: ' ' })).toThrow('unavailable');
    expect(raw.configure).not.toHaveBeenCalled();
  });

  it('logs in with the suite subject and logs out only identified customers', async () => {
    const { module, raw, calls } = mockPurchases();
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });

    await port.reset();
    expect(raw.logOut).not.toHaveBeenCalled();
    await port.identify(SUBJECT);
    await port.reset();

    expect(calls).toEqual([
      `configure:${JSON.stringify({ apiKey: API_KEY })}`,
      `logIn:${JSON.stringify(SUBJECT)}`,
      'logOut',
    ]);
  });

  it('maps offerings and purchases the matching native package', async () => {
    const { module, raw } = mockPurchases();
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });

    const offerings = await port.offerings();

    expect(offerings.current?.id).toBe('default');
    expect(offerings.all['packs']?.packages).toEqual([
      {
        id: 'small',
        offeringId: 'packs',
        productId: 'app.pack.small',
        kind: 'one_time',
        title: 'app.pack.small title',
        description: 'app.pack.small description',
        priceLabel: '€4.99',
      },
    ]);
    const subscription = firstPackage(offerings);
    expect(subscription.kind).toBe('subscription');
    await port.purchase(subscription);
    expect(raw.purchasePackage).toHaveBeenCalledWith(monthly);
  });

  it('treats an offering without a current entry as none', async () => {
    const { module, raw } = mockPurchases();
    raw.getOfferings.mockResolvedValueOnce({ ...OFFERINGS, current: null });
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });

    await expect(port.offerings()).resolves.toMatchObject({ current: null });
  });

  it('rejects a package that the last offerings did not contain', async () => {
    const { module, raw } = mockPurchases();
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });
    const offerings = await port.offerings();
    const item = firstPackage(offerings);
    await port.reset();

    await expect(port.purchase(item)).rejects.toMatchObject({ code: 'store' });
    await expect(port.purchase({ ...item, id: 'unknown' })).rejects.toMatchObject({
      code: 'store',
    });
    expect(raw.purchasePackage).not.toHaveBeenCalled();
  });

  it.each([
    [{ code: '1' }, 'cancelled'],
    [{ code: '20' }, 'pending'],
    [{ code: '10' }, 'network'],
    [{ code: '35' }, 'network'],
    [{ code: '2' }, 'store'],
    ['opaque', 'store'],
  ])('maps RevenueCat error %j to %s', async (error, code) => {
    const { module, raw } = mockPurchases();
    raw.restorePurchases.mockRejectedValueOnce(error);
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });

    await expect(port.restore()).rejects.toMatchObject({ code, cause: error });
  });

  it('forwards and removes customer info listeners', () => {
    const { module, raw, emit } = mockPurchases();
    const port = createRevenueCatPurchasesPort(module, { apiKey: API_KEY });
    const listener = vi.fn();

    const stop = port.onCustomerUpdate(listener);
    emit();
    stop();
    emit();

    expect(listener).toHaveBeenCalledTimes(1);
    expect(raw.removeCustomerInfoUpdateListener).toHaveBeenCalledTimes(1);
  });

  it('logs in before the flow buys and logs out on sign-out', async () => {
    const { module, calls } = mockPurchases();
    let revision = 1;
    const flow = new PurchaseFlow({
      port: createRevenueCatPurchasesPort(module, { apiKey: API_KEY }),
      backend: {
        refreshEntitlement: () =>
          Promise.resolve({ suiteSubject: SUBJECT, tier: 'premium', features: [], revision }),
      },
      sleep: () => {
        revision = 2;
        return Promise.resolve();
      },
    });
    const offerings = await flow.offerings();
    const item = firstPackage(offerings);

    await expect(flow.purchase(item)).rejects.toMatchObject({ code: 'not_identified' });
    await flow.identify(SUBJECT);
    await expect(flow.purchase(item)).resolves.toMatchObject({ status: 'granted' });
    await flow.signOut();

    expect(calls).toEqual([
      `configure:${JSON.stringify({ apiKey: API_KEY })}`,
      'getOfferings',
      `logIn:${JSON.stringify(SUBJECT)}`,
      'purchasePackage:$rc_monthly',
      'logOut',
    ]);
  });
});
