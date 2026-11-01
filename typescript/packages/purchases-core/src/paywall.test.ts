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
const context = { productId: 'eigenruhe', feature: 'eigenruhe.studio' };
const link = `${PLANS}?from=eigenruhe&feature=eigenruhe.studio`;

describe('plansLink', () => {
  it('adds the product and feature to the C17 plans URL', () => {
    expect(plansLink(PLANS, { from: 'eigenruhe', feature: 'eigenruhe.studio' })).toBe(link);
    expect(plansLink(PLANS, { from: 'hebkit' })).toBe(`${PLANS}?from=hebkit`);
    expect(plansLink(PLANS)).toBe(PLANS);
    expect(plansLink(`${PLANS}?lang=de`, { feature: 'a b&c' })).toBe(
      `${PLANS}?lang=de&feature=a%20b%26c`,
    );
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
    expect(paywallFor(plan({ channel: 'app_store', soldBy: 'hebkit' }), context)).toEqual({
      kind: 'manage',
      channel: 'app_store',
      manageUrl: null,
      soldBy: 'hebkit',
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
