import type { BillingChannel, VerifiedEntitlement } from './entitlement.js';
import { plansLink } from './plans-link.js';

/** What a locked feature may offer. Only `purchase` sells anything. */
export type Paywall =
  | { readonly kind: 'purchase'; readonly plansUrl: string | null }
  | {
      readonly kind: 'manage';
      readonly channel: 'web' | 'app_store' | 'play_store';
      readonly manageUrl: string | null;
      readonly soldBy: string | null;
    }
  | { readonly kind: 'covered'; readonly channel: 'family' | 'organization' }
  | {
      readonly kind: 'organization_managed';
      readonly organizationId: string;
      readonly organizationName: string | null;
    }
  | { readonly kind: 'none' };

export interface PaywallContext {
  /** The calling product, sent as `from` on the Plans link. */
  readonly productId: string;
  readonly feature?: string;
}

/**
 * The C18 rules: a plan managed on the web or in a store shows where to manage it and
 * never sells a second plan, family and organization coverage offer no purchase, and
 * standalone mode sells nothing. Promo grants and free accounts may buy. An organization's
 * plan covers its members, and a feature it lacks names the organization whose admins manage
 * the plan instead of offering a purchase.
 */
export function paywallFor(entitlement: VerifiedEntitlement, context: PaywallContext): Paywall {
  if (entitlement.mode === 'standalone') return { kind: 'none' };
  const organization = entitlement.organization;
  if (organization) {
    if (context.feature === undefined || entitlement.features.includes(context.feature))
      return { kind: 'covered', channel: 'organization' };
    return {
      kind: 'organization_managed',
      organizationId: organization.organizationId,
      organizationName: organization.name,
    };
  }
  const link =
    entitlement.plansUrl === null
      ? null
      : plansLink(entitlement.plansUrl, {
          from: context.productId,
          ...(context.feature === undefined ? {} : { feature: context.feature }),
        });
  const channel: BillingChannel | undefined = entitlement.billing?.channel;
  switch (channel) {
    case 'web':
      return {
        kind: 'manage',
        channel,
        manageUrl: link ?? entitlement.billing?.manageUrl ?? null,
        soldBy: null,
      };
    case 'app_store':
    case 'play_store':
      return {
        kind: 'manage',
        channel,
        manageUrl: null,
        soldBy: entitlement.billing?.soldBy ?? null,
      };
    case 'family':
    case 'organization':
      return { kind: 'covered', channel };
    case 'promo':
    case undefined:
      return { kind: 'purchase', plansUrl: link };
  }
}
