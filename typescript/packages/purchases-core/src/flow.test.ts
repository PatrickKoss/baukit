import { describe, expect, it, vi } from 'vitest';

import {
  createNoopPurchasesPort,
  entitlementFromTicket,
  hasFeature,
  PurchaseError,
  PurchaseFlow,
  selectOffering,
  type CreditBalanceSnapshot,
  type EntitlementBackend,
  type PurchaseOfferings,
  type PurchasePackage,
  type PurchasesPort,
  type VerifiedEntitlement,
} from './index.js';

const SUBJECT = 'suite|person-1';
const OTHER = 'suite|person-2';

const monthly: PurchasePackage = {
  id: '$rc_monthly',
  offeringId: 'default',
  productId: 'app.product.monthly',
  kind: 'subscription',
  title: 'Premium',
  description: 'Monthly',
  priceLabel: '€4.99',
};
const pack: PurchasePackage = { ...monthly, id: 'pack_small', kind: 'one_time' };

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((onResolve) => {
    resolve = onResolve;
  });
  return { promise, resolve };
}

class FakePort implements PurchasesPort {
  readonly calls: string[] = [];
  identifyGate: Promise<void> | null = null;
  purchaseError: Error | null = null;
  onPurchase: () => void = () => undefined;
  #listener: (() => void) | null = null;

  async identify(subject: string): Promise<void> {
    this.calls.push(`identify:${subject}`);
    await this.identifyGate;
  }
  reset(): Promise<void> {
    this.calls.push('reset');
    return Promise.resolve();
  }
  offerings(): Promise<PurchaseOfferings> {
    return Promise.resolve({
      current: { id: 'default', packages: [monthly] },
      all: { default: { id: 'default', packages: [monthly] } },
    });
  }
  purchase(item: PurchasePackage): Promise<void> {
    this.calls.push(`purchase:${item.id}`);
    if (this.purchaseError !== null) return Promise.reject(this.purchaseError);
    this.onPurchase();
    return Promise.resolve();
  }
  restore(): Promise<void> {
    this.calls.push('restore');
    this.onPurchase();
    return Promise.resolve();
  }
  onCustomerUpdate(listener: () => void): () => void {
    this.#listener = listener;
    return () => {
      this.#listener = null;
    };
  }
  emitCustomerUpdate(): void {
    this.#listener?.();
  }
}

class FakeBackend implements EntitlementBackend {
  entitlement: VerifiedEntitlement | null = entitlement(1, []);
  balance: CreditBalanceSnapshot | null = { suiteSubject: SUBJECT, available: 0, revision: 0 };
  refreshes = 0;
  failNext = false;

  refreshEntitlement(): Promise<VerifiedEntitlement | null> {
    this.refreshes += 1;
    if (this.failNext) {
      this.failNext = false;
      return Promise.reject(new Error('backend offline'));
    }
    return Promise.resolve(this.entitlement);
  }
  creditBalance(): Promise<CreditBalanceSnapshot | null> {
    return Promise.resolve(this.balance);
  }
}

function entitlement(
  revision: number,
  features: string[],
  suiteSubject = SUBJECT,
): VerifiedEntitlement {
  return { suiteSubject, tier: features.length ? 'premium' : 'free', features, revision };
}

function setup(attempts = 3) {
  const port = new FakePort();
  const backend = new FakeBackend();
  const sleep = vi.fn(() => Promise.resolve());
  const flow = new PurchaseFlow({ port, backend, refreshAttempts: attempts, sleep });
  return { port, backend, sleep, flow };
}

describe('PurchaseFlow identity', () => {
  it('refuses to purchase before identify', async () => {
    const { port, flow } = setup();

    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'not_identified' });
    await expect(flow.restore()).rejects.toMatchObject({ code: 'not_identified' });
    expect(port.calls).toEqual([]);
  });

  it('refuses to purchase while identify is still running', async () => {
    const { port, flow } = setup();
    const gate = deferred();
    port.identifyGate = gate.promise;

    const identifying = flow.identify(SUBJECT);
    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'not_identified' });
    gate.resolve();
    await identifying;

    expect(port.calls).toEqual([`identify:${SUBJECT}`]);
    expect(flow.getState().suiteSubject).toBe(SUBJECT);
  });

  it('stays signed out when the store rejects identify', async () => {
    const { port, flow } = setup();
    port.identify = () => Promise.reject(new PurchaseError('network'));

    await expect(flow.identify(SUBJECT)).rejects.toMatchObject({ code: 'network' });
    expect(flow.getState().suiteSubject).toBeNull();
  });

  it('rejects an empty subject', async () => {
    const { port, flow } = setup();

    await expect(flow.identify(' ')).rejects.toMatchObject({ code: 'invalid_subject' });
    expect(port.calls).toEqual([]);
  });

  it('drops an identify that a sign-out overtook', async () => {
    const { port, flow } = setup();
    const gate = deferred();
    port.identifyGate = gate.promise;

    const identifying = flow.identify(SUBJECT);
    await flow.signOut();
    gate.resolve();

    await expect(identifying).rejects.toMatchObject({ code: 'identity_changed' });
    expect(flow.getState().suiteSubject).toBeNull();
  });

  it('resets the store and clears state on sign-out', async () => {
    const { port, backend, flow } = setup();
    backend.entitlement = entitlement(4, ['premium']);
    await flow.identify(SUBJECT);
    await flow.refreshEntitlement();

    await flow.signOut();

    expect(port.calls).toEqual([`identify:${SUBJECT}`, 'reset']);
    expect(flow.getState()).toEqual({ suiteSubject: null, entitlement: null, creditBalance: null });
    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'not_identified' });
  });

  it('rejects a purchase result when the user signed out meanwhile', async () => {
    const { port, flow } = setup();
    await flow.identify(SUBJECT);
    port.onPurchase = () => {
      void flow.signOut();
    };

    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'identity_changed' });
  });
});

describe('PurchaseFlow purchase', () => {
  it('identifies, purchases and shows the refreshed entitlement', async () => {
    const { port, backend, flow, sleep } = setup();
    await flow.identify(SUBJECT);
    port.onPurchase = () => {
      backend.entitlement = entitlement(2, ['premium']);
    };

    const outcome = await flow.purchase(monthly);

    expect(port.calls).toEqual([`identify:${SUBJECT}`, 'purchase:$rc_monthly']);
    expect(outcome).toEqual({ status: 'granted', entitlement: entitlement(2, ['premium']) });
    expect(flow.getState().entitlement).toEqual(entitlement(2, ['premium']));
    expect(backend.refreshes).toBe(2);
    expect(sleep).not.toHaveBeenCalled();
  });

  it('waits for the ticket revision that follows the purchase', async () => {
    const { port, backend, flow, sleep } = setup(4);
    await flow.identify(SUBJECT);
    let reads = 0;
    port.onPurchase = () => {
      backend.refreshEntitlement = () => {
        reads += 1;
        if (reads === 1) return Promise.reject(new Error('backend offline'));
        return Promise.resolve(reads < 3 ? entitlement(1, []) : entitlement(2, ['premium']));
      };
    };

    const outcome = await flow.purchase(monthly);

    expect(outcome.status).toBe('granted');
    expect(reads).toBe(3);
    expect(sleep).toHaveBeenCalledTimes(2);
    expect(sleep).toHaveBeenCalledWith(2_000);
  });

  it('reports pending when the refreshed entitlement has not arrived', async () => {
    const { backend, flow, sleep } = setup(3);
    await flow.identify(SUBJECT);

    const outcome = await flow.purchase(monthly);

    expect(outcome).toEqual({ status: 'pending', entitlement: entitlement(1, []) });
    expect(backend.refreshes).toBe(4);
    expect(sleep).toHaveBeenCalledTimes(2);
  });

  it('reads a fresh baseline so a change that landed before the purchase does not grant it', async () => {
    const { port, backend, flow } = setup(3);
    await flow.identify(SUBJECT);
    backend.entitlement = entitlement(2, ['extra']);
    port.onPurchase = () => {
      backend.entitlement = entitlement(3, ['premium']);
    };

    await expect(flow.purchase(monthly)).resolves.toEqual({
      status: 'granted',
      entitlement: entitlement(3, ['premium']),
    });
  });

  it('stays pending when only the change before the purchase arrives', async () => {
    const { backend, flow } = setup(3);
    await flow.identify(SUBJECT);
    backend.entitlement = entitlement(2, ['extra']);

    await expect(flow.purchase(monthly)).resolves.toEqual({
      status: 'pending',
      entitlement: entitlement(2, ['extra']),
    });
  });

  it('rejects before the store call when the baseline read fails', async () => {
    const { port, backend, flow } = setup();
    await flow.identify(SUBJECT);
    backend.failNext = true;

    await expect(flow.purchase(monthly)).rejects.toThrow('backend offline');
    expect(port.calls).toEqual([`identify:${SUBJECT}`]);
  });

  it('passes a cancelled purchase through without refreshing', async () => {
    const { port, backend, flow } = setup();
    await flow.identify(SUBJECT);
    port.purchaseError = new PurchaseError('cancelled');

    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'cancelled' });
    expect(backend.refreshes).toBe(1);
  });

  it('passes a store-pending purchase through', async () => {
    const { port, flow } = setup();
    await flow.identify(SUBJECT);
    port.purchaseError = new PurchaseError('pending');

    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'pending' });
  });

  it('wraps unknown store failures', async () => {
    const { port, flow } = setup();
    await flow.identify(SUBJECT);
    const cause = new Error('boom');
    port.purchaseError = cause;

    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'store', cause });
  });

  it('refuses a second purchase while one runs', async () => {
    const { port, flow } = setup(1);
    await flow.identify(SUBJECT);
    const gate = deferred();
    port.purchase = async () => {
      await gate.promise;
    };

    const first = flow.purchase(monthly);
    await Promise.resolve();
    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'busy' });
    gate.resolve();
    await expect(first).resolves.toMatchObject({ status: 'pending' });
  });

  it('rejects an entitlement for another subject', async () => {
    const { backend, flow } = setup();
    await flow.identify(SUBJECT);
    backend.entitlement = entitlement(3, ['premium'], OTHER);

    await expect(flow.refreshEntitlement()).rejects.toMatchObject({ code: 'subject_mismatch' });
    expect(flow.getState().entitlement).toBeNull();
  });

  it('keeps a newer entitlement over a late older one', async () => {
    const { backend, flow } = setup();
    await flow.identify(SUBJECT);
    backend.entitlement = entitlement(5, ['premium']);
    await flow.refreshEntitlement();
    backend.entitlement = entitlement(4, []);

    await expect(flow.refreshEntitlement()).resolves.toEqual(entitlement(5, ['premium']));
  });

  it('restores and reports unchanged when nothing new arrives', async () => {
    const { port, backend, flow } = setup(2);
    await flow.identify(SUBJECT);

    await expect(flow.restore()).resolves.toEqual({
      status: 'unchanged',
      entitlement: entitlement(1, []),
    });

    port.onPurchase = () => {
      backend.entitlement = entitlement(2, ['premium']);
    };
    await expect(flow.restore()).resolves.toEqual({
      status: 'granted',
      entitlement: entitlement(2, ['premium']),
    });
  });

  it('refreshes the entitlement on a customer update', async () => {
    const { port, backend, flow } = setup();
    port.emitCustomerUpdate();
    expect(backend.refreshes).toBe(0);
    await flow.identify(SUBJECT);
    backend.entitlement = entitlement(2, ['premium']);

    port.emitCustomerUpdate();
    await vi.waitFor(() => {
      expect(flow.getState().entitlement?.revision).toBe(2);
    });

    backend.failNext = true;
    port.emitCustomerUpdate();
    await vi.waitFor(() => {
      expect(backend.refreshes).toBe(2);
    });
    flow.dispose();
    port.emitCustomerUpdate();
    expect(backend.refreshes).toBe(2);
  });

  it('notifies subscribers until they unsubscribe', async () => {
    const { flow } = setup();
    const listener = vi.fn();
    const unsubscribe = flow.subscribe(listener);

    await flow.identify(SUBJECT);
    unsubscribe();
    await flow.refreshEntitlement();

    expect(listener).toHaveBeenCalledTimes(2);
  });
});

describe('PurchaseFlow credit packs', () => {
  it('purchases a pack and reports the credited balance', async () => {
    const { port, backend, flow } = setup();
    await flow.identify(SUBJECT);
    port.onPurchase = () => {
      backend.balance = { suiteSubject: SUBJECT, available: 1_000, revision: 1 };
    };

    const outcome = await flow.purchaseCreditPack(pack);

    expect(port.calls).toEqual([`identify:${SUBJECT}`, 'purchase:pack_small']);
    expect(outcome).toEqual({
      status: 'credited',
      balance: { suiteSubject: SUBJECT, available: 1_000, revision: 1 },
    });
    expect(flow.getState().creditBalance?.available).toBe(1_000);
  });

  it('reports a pack as pending until the balance revision moves', async () => {
    const { flow } = setup(2);
    await flow.identify(SUBJECT);

    await expect(flow.purchaseCreditPack(pack)).resolves.toEqual({
      status: 'pending',
      balance: { suiteSubject: SUBJECT, available: 0, revision: 0 },
    });
  });

  it('reads a fresh balance baseline before buying a pack', async () => {
    const { port, backend, flow } = setup(3);
    await flow.identify(SUBJECT);
    await flow.refreshCreditBalance();
    backend.balance = { suiteSubject: SUBJECT, available: 500, revision: 1 };
    port.onPurchase = () => {
      backend.balance = { suiteSubject: SUBJECT, available: 1_500, revision: 2 };
    };

    await expect(flow.purchaseCreditPack(pack)).resolves.toEqual({
      status: 'credited',
      balance: { suiteSubject: SUBJECT, available: 1_500, revision: 2 },
    });
  });

  it('keeps a pack pending when only the earlier balance change arrives', async () => {
    const { backend, flow } = setup(3);
    await flow.identify(SUBJECT);
    await flow.refreshCreditBalance();
    backend.balance = { suiteSubject: SUBJECT, available: 500, revision: 1 };

    await expect(flow.purchaseCreditPack(pack)).resolves.toEqual({
      status: 'pending',
      balance: { suiteSubject: SUBJECT, available: 500, revision: 1 },
    });
  });

  it('rejects a pack before the store call when the balance read fails', async () => {
    const { port, backend, flow } = setup();
    await flow.identify(SUBJECT);
    backend.creditBalance = () => Promise.reject(new Error('backend offline'));

    await expect(flow.purchaseCreditPack(pack)).rejects.toThrow('backend offline');
    expect(port.calls).toEqual([`identify:${SUBJECT}`]);
  });

  it('treats a missing balance as no revision yet', async () => {
    const { port, backend, flow } = setup(1);
    await flow.identify(SUBJECT);
    backend.balance = null;
    port.onPurchase = () => {
      backend.balance = { suiteSubject: SUBJECT, available: 3_000, revision: 0 };
    };

    await expect(flow.purchaseCreditPack(pack)).resolves.toMatchObject({ status: 'credited' });
  });

  it('refuses subscriptions and packs without identity or a balance source', async () => {
    const port = new FakePort();
    const flow = new PurchaseFlow({
      port,
      backend: { refreshEntitlement: () => Promise.resolve(null) },
    });

    await expect(flow.purchaseCreditPack(monthly)).rejects.toMatchObject({
      code: 'not_credit_pack',
    });
    await expect(flow.purchaseCreditPack(pack)).rejects.toMatchObject({ code: 'not_identified' });
    await flow.identify(SUBJECT);
    await expect(flow.purchaseCreditPack(pack)).rejects.toMatchObject({ code: 'unavailable' });
    expect(port.calls).toEqual([`identify:${SUBJECT}`]);
  });
});

describe('no-op purchases', () => {
  it('identifies, sells nothing and keeps gating on the backend entitlement', async () => {
    const backend = new FakeBackend();
    backend.entitlement = entitlement(3, ['premium']);
    const flow = new PurchaseFlow({ port: createNoopPurchasesPort(), backend });

    await flow.identify(SUBJECT);
    await flow.refreshEntitlement();
    await expect(flow.offerings()).resolves.toEqual({ current: null, all: {} });
    await expect(flow.purchase(monthly)).rejects.toMatchObject({ code: 'unavailable' });
    await expect(flow.restore()).rejects.toMatchObject({ code: 'unavailable' });
    expect(hasFeature(flow.getState().entitlement, 'premium')).toBe(true);
    await flow.signOut();
    expect(flow.getState().entitlement).toBeNull();
  });
});

describe('helpers', () => {
  it('selects the configured offering or the current one', async () => {
    const offerings = await new FakePort().offerings();

    expect(selectOffering(offerings)?.id).toBe('default');
    expect(selectOffering(offerings, 'default')?.id).toBe('default');
    expect(selectOffering(offerings, 'missing')).toBeNull();
  });

  it('maps verified ticket claims and gates on feature keys', () => {
    const verified = entitlementFromTicket({
      sub: SUBJECT,
      tier: 'premium',
      features: ['export'],
      rev: 7,
    });

    expect(verified).toEqual({
      suiteSubject: SUBJECT,
      tier: 'premium',
      features: ['export'],
      revision: 7,
    });
    expect(hasFeature(verified, 'export')).toBe(true);
    expect(hasFeature(verified, 'sync')).toBe(false);
    expect(hasFeature(null, 'export')).toBe(false);
  });

  it('names its errors', () => {
    const error = new PurchaseError('network');

    expect(error).toBeInstanceOf(Error);
    expect(error.name).toBe('PurchaseError');
    expect(error.message).toBe('network');
  });
});
