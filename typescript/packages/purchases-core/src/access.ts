import type { VerifiedEntitlement } from './entitlement.js';
import type { PurchaseState } from './flow.js';
import { paywallFor, type Paywall, type PaywallContext } from './paywall.js';

/**
 * Gating is closed until a verified entitlement grants the key. `loading` and
 * `unavailable` show neither the feature nor a paywall, so a slow or failing
 * backend never unlocks a feature and never sells a plan.
 */
export type FeatureAccess =
  | { readonly status: 'loading' }
  | { readonly status: 'unavailable' }
  | { readonly status: 'granted'; readonly entitlement: VerifiedEntitlement }
  | {
      readonly status: 'locked';
      readonly entitlement: VerifiedEntitlement | null;
      readonly paywall: Paywall;
    };

export function featureAccess(
  state: PurchaseState,
  feature: string,
  context: Omit<PaywallContext, 'feature'>,
): FeatureAccess {
  const { entitlement, entitlementStatus } = state;
  if (entitlement?.features.includes(feature)) {
    return { status: 'granted', entitlement };
  }
  if (entitlement !== null) {
    return {
      status: 'locked',
      entitlement,
      paywall: paywallFor(entitlement, { ...context, feature }),
    };
  }
  switch (entitlementStatus) {
    case 'loading':
      return { status: 'loading' };
    case 'unavailable':
      return { status: 'unavailable' };
    case 'signed_out':
    case 'ready':
      return { status: 'locked', entitlement: null, paywall: { kind: 'none' } };
  }
}
