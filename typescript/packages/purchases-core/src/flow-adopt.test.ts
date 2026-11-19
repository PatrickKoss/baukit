import { describe, expect, it } from 'vitest';

import {
  createNoopPurchasesPort,
  PurchaseFlow,
  type EntitlementBackend,
  type VerifiedEntitlement,
} from './index.js';

const SUBJECT = 'suite|person-1';
const OTHER = 'suite|person-2';

function entitlement(revision: number, suiteSubject = SUBJECT): VerifiedEntitlement {
  return {
    mode: 'hub',
    suiteSubject,
    tier: revision > 1 ? 'premium' : 'free',
    features: revision > 1 ? ['export'] : [],
    revision,
    plansUrl: null,
    billing: null,
  };
}

class GatedBackend implements EntitlementBackend {
  refreshes = 0;
  #pending: ((value: VerifiedEntitlement | null) => void) | null = null;

  refreshEntitlement(): Promise<VerifiedEntitlement | null> {
    this.refreshes += 1;
    return new Promise((resolve) => {
      this.#pending = resolve;
    });
  }

  answer(value: VerifiedEntitlement | null): void {
    this.#pending?.(value);
  }
}

async function identifiedFlow(): Promise<{ flow: PurchaseFlow; backend: GatedBackend }> {
  const backend = new GatedBackend();
  const flow = new PurchaseFlow({ port: createNoopPurchasesPort(), backend });
  await flow.identify(SUBJECT);
  return { flow, backend };
}

describe('PurchaseFlow.adoptEntitlement', () => {
  it('makes a fetched entitlement ready without a backend read', async () => {
    const { flow, backend } = await identifiedFlow();

    expect(flow.adoptEntitlement(entitlement(2))).toEqual(entitlement(2));

    expect(flow.getState()).toMatchObject({
      entitlementStatus: 'ready',
      entitlement: entitlement(2),
    });
    expect(backend.refreshes).toBe(0);
  });

  it('refuses an entitlement before identify', () => {
    const flow = new PurchaseFlow({ port: createNoopPurchasesPort(), backend: new GatedBackend() });

    expect(() => flow.adoptEntitlement(entitlement(1))).toThrow('not_identified');
    expect(flow.getState().entitlementStatus).toBe('signed_out');
  });

  it('refuses an entitlement of another subject', async () => {
    const { flow } = await identifiedFlow();

    expect(() => flow.adoptEntitlement(entitlement(1, OTHER))).toThrow('subject_mismatch');
    expect(flow.getState()).toMatchObject({ entitlementStatus: 'loading', entitlement: null });
  });

  it('refuses a malformed body', async () => {
    const { flow } = await identifiedFlow();

    expect(() => flow.adoptEntitlement({ ...entitlement(1), revision: -1 })).toThrow(TypeError);
    expect(() => flow.adoptEntitlement(null)).toThrow(TypeError);
    expect(flow.getState().entitlement).toBeNull();
  });

  it('keeps a newer entitlement when handed stale data', async () => {
    const { flow } = await identifiedFlow();
    flow.adoptEntitlement(entitlement(3));

    expect(flow.adoptEntitlement(entitlement(2))).toEqual(entitlement(3));
    expect(flow.getState().entitlement).toEqual(entitlement(3));
  });

  it('keeps the newest revision when it races an in-flight refresh', async () => {
    const { flow, backend } = await identifiedFlow();
    const older = flow.refreshEntitlement();
    flow.adoptEntitlement(entitlement(2));
    backend.answer(entitlement(1));
    await expect(older).resolves.toEqual(entitlement(2));

    const newer = flow.refreshEntitlement();
    flow.adoptEntitlement(entitlement(2));
    backend.answer(entitlement(4));
    await expect(newer).resolves.toEqual(entitlement(4));
    expect(flow.getState().entitlement).toEqual(entitlement(4));
  });

  it('drops an entitlement adopted for an identity that changed during a refresh', async () => {
    const { flow, backend } = await identifiedFlow();
    const refresh = flow.refreshEntitlement();
    await flow.identify(OTHER);

    expect(() => flow.adoptEntitlement(entitlement(1))).toThrow('subject_mismatch');
    backend.answer(entitlement(1));
    await expect(refresh).rejects.toThrow('identity_changed');
    expect(flow.getState()).toMatchObject({ suiteSubject: OTHER, entitlement: null });
  });

  it('does nothing after dispose', async () => {
    const { flow } = await identifiedFlow();
    let notified = 0;
    flow.subscribe(() => {
      notified += 1;
    });
    flow.dispose();

    expect(flow.adoptEntitlement(entitlement(2))).toBeNull();
    expect(flow.getState()).toMatchObject({ entitlementStatus: 'loading', entitlement: null });
    expect(notified).toBe(0);
  });
});
