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

/** An organization's plan as a client loads it through `organizationEntitlement`. */
export type OrganizationPlanState =
  | { readonly status: 'loading' }
  | { readonly status: 'unavailable' }
  | { readonly status: 'not_accessible' }
  | { readonly status: 'ready'; readonly entitlement: VerifiedEntitlement };

/** Access to `feature` through an organization's plan. A non-member sees neither feature nor paywall. */
export function organizationFeatureAccess(
  organization: OrganizationPlanState,
  feature: string,
  context: Omit<PaywallContext, 'feature'>,
): FeatureAccess {
  switch (organization.status) {
    case 'loading':
    case 'unavailable':
      return { status: organization.status };
    case 'not_accessible':
      return { status: 'locked', entitlement: null, paywall: { kind: 'none' } };
    case 'ready': {
      const { entitlement } = organization;
      if (entitlement.features.includes(feature)) return { status: 'granted', entitlement };
      return {
        status: 'locked',
        entitlement,
        paywall: paywallFor(entitlement, { ...context, feature }),
      };
    }
  }
}

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
