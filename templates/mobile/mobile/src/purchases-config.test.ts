import { describe, expect, it, jest } from '@jest/globals';
import type { PurchasesPort } from '@baukit/purchases-core';

import { createPurchasesPort, readPurchasesConfig } from './purchases-config';

const extra = {
  revenueCat: { iosApiKey: ' appl_key ', androidApiKey: 'goog_key', offeringId: 'default' },
};

describe('purchases configuration', () => {
  it('reads the key for the running platform', () => {
    expect(readPurchasesConfig(extra, 'ios')).toEqual({ apiKey: 'appl_key', offeringId: 'default' });
    expect(readPurchasesConfig(extra, 'android').apiKey).toBe('goog_key');
  });

  it('has no key on web, without config, or with blank values', () => {
    expect(readPurchasesConfig(extra, 'web').apiKey).toBeNull();
    expect(readPurchasesConfig(undefined, 'ios')).toEqual({ apiKey: null, offeringId: null });
    expect(readPurchasesConfig({ revenueCat: { iosApiKey: ' ', offeringId: 3 } }, 'ios')).toEqual({
      apiKey: null,
      offeringId: null,
    });
  });

  it('uses the no-op port without a key', async () => {
    const revenueCat = jest.fn<(apiKey: string) => PurchasesPort>();

    const port = createPurchasesPort({ apiKey: null, offeringId: null }, revenueCat);

    expect(revenueCat).not.toHaveBeenCalled();
    await expect(port.offerings()).resolves.toEqual({ current: null, all: {} });
    await expect(port.purchase({} as never)).rejects.toMatchObject({ code: 'unavailable' });
  });

  it('builds the RevenueCat port with the configured key', () => {
    const configured = {} as PurchasesPort;
    const revenueCat = jest.fn<(apiKey: string) => PurchasesPort>(() => configured);

    expect(createPurchasesPort({ apiKey: 'goog_key', offeringId: null }, revenueCat)).toBe(configured);
    expect(revenueCat).toHaveBeenCalledWith('goog_key');
  });
});
