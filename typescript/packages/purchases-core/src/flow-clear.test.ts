import { describe, expect, it } from 'vitest';
import type { VerifiedEntitlement } from './entitlement.js';
import { PurchaseFlow } from './flow.js';
import { createNoopPurchasesPort } from './noop.js';

const SUBJECT = 'suite:alice';
const paid: VerifiedEntitlement = {
  mode: 'hub',
  suiteSubject: SUBJECT,
  tier: 'premium',
  features: ['export'],
  revision: 4,
  plansUrl: null,
  billing: null,
};

describe('a refresh without a plan', () => {
  it('drops the grant held from an earlier answer', async () => {
    let answer: VerifiedEntitlement | null = paid;
    const flow = new PurchaseFlow({
      port: createNoopPurchasesPort(),
      backend: { refreshEntitlement: () => Promise.resolve(answer) },
    });
    await flow.identify(SUBJECT);
    await flow.refreshEntitlement();
    answer = null;
    await expect(flow.refreshEntitlement()).resolves.toBeNull();
    expect(flow.getState().entitlement).toBeNull();
  });
});
