import { describe, expect, it } from 'vitest';

import { paywallFor, plansLink, type BillingClaim, type VerifiedEntitlement } from './index.js';

const PLANS = 'https://accounts.example.test/plans';

function plan(
  billing: BillingClaim | null,
  mode: VerifiedEntitlement['mode'] = 'hub',
): VerifiedEntitlement {
  return {
    mode,
    suiteSubject: 'suite|a',
    tier: 'premium',
    features: [],
    revision: 1,
    plansUrl: mode === 'hub' ? PLANS : null,
    billing,
  };
}
const context = { productId: 'beta', feature: 'beta.studio' };
const link = `${PLANS}?from=beta&feature=beta.studio`;

describe('plansLink', () => {
  it('adds the product and feature to the hub plans URL', () => {
    expect(plansLink(PLANS, { from: 'beta', feature: 'beta.studio' })).toBe(link);
    expect(plansLink(PLANS, { from: 'alpha' })).toBe(`${PLANS}?from=alpha`);
    expect(plansLink(PLANS)).toBe(PLANS);
    expect(plansLink(`${PLANS}?lang=de`, { from: 'alpha', feature: 'a b&c' })).toBe(
      `${PLANS}?lang=de&from=alpha&feature=a%20b%26c`,
    );
    expect(plansLink(PLANS, { feature: 'beta.studio' })).toBe(PLANS);
  });

  it('refuses a URL that is not HTTP(S)', () => {
    for (const url of ['/plans', 'javascript:alert(1)', 'https://user@host/plans'])
      expect(() => plansLink(url)).toThrow(TypeError);
  });
});

describe('paywallFor', () => {
  it('sells to free and promo accounts through the plans link', () => {
    expect(paywallFor(plan(null), context)).toEqual({ kind: 'purchase', plansUrl: link });
    expect(paywallFor(plan({ channel: 'promo' }), context)).toEqual({
      kind: 'purchase',
      plansUrl: link,
    });
  });

  it('shows where a web or store plan is managed and never sells a second plan', () => {
    expect(paywallFor(plan({ channel: 'web', manageUrl: PLANS }), context)).toEqual({
      kind: 'manage',
      channel: 'web',
      manageUrl: link,
      soldBy: null,
    });
    expect(paywallFor(plan({ channel: 'app_store', soldBy: 'alpha' }), context)).toEqual({
      kind: 'manage',
      channel: 'app_store',
      manageUrl: null,
      soldBy: 'alpha',
    });
    expect(paywallFor(plan({ channel: 'play_store' }), context)).toEqual({
      kind: 'manage',
      channel: 'play_store',
      manageUrl: null,
      soldBy: null,
    });
  });

  it('offers no purchase under family or organization coverage or in standalone mode', () => {
    expect(paywallFor(plan({ channel: 'family' }), context)).toEqual({
      kind: 'covered',
      channel: 'family',
    });
    expect(paywallFor(plan({ channel: 'organization' }), context)).toEqual({
      kind: 'covered',
      channel: 'organization',
    });
    expect(paywallFor(plan(null, 'standalone'), context)).toEqual({ kind: 'none' });
  });
});
