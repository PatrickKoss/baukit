export type PlanMode = 'hub' | 'standalone';

export type BillingChannel =
  'web' | 'app_store' | 'play_store' | 'family' | 'organization' | 'promo';

/** The hub ticket's optional `billing` claim, passed through by the product backend. */
export interface BillingClaim {
  readonly channel: BillingChannel;
  readonly soldBy?: string;
  readonly manageUrl?: string;
}

/** The body of the product backend's `GET /me/entitlement`. */
export interface VerifiedEntitlement {
  readonly mode: PlanMode;
  readonly suiteSubject: string;
  readonly tier: string;
  readonly features: readonly string[];
  readonly revision: number;
  /** The hub `/plans` page in hub mode, null in standalone mode. */
  readonly plansUrl: string | null;
  readonly billing: BillingClaim | null;
}

/** The body of the product backend's `GET /me/credits`. */
export interface CreditBalanceSnapshot {
  readonly suiteSubject: string;
  readonly wallet: string;
  readonly revision: number;
  readonly total: number;
  readonly grant: { readonly available: number; readonly expiresAt: number | null };
  readonly packs: readonly { readonly available: number; readonly expiresAt: number }[];
}

/** Product backend calls. `null` means the session has no suite identity. */
export interface EntitlementBackend {
  refreshEntitlement(): Promise<VerifiedEntitlement | null>;
  creditBalance?(): Promise<CreditBalanceSnapshot | null>;
}

export interface TicketClaimsLike {
  readonly sub: string;
  readonly tier: string;
  readonly features: readonly string[];
  readonly rev: number;
  readonly billing?: BillingClaim;
}

export function entitlementFromTicket(
  claims: TicketClaimsLike,
  plansUrl: string | null = null,
): VerifiedEntitlement {
  return {
    mode: 'hub',
    suiteSubject: claims.sub,
    tier: claims.tier,
    features: claims.features,
    revision: claims.rev,
    plansUrl,
    billing: claims.billing ?? null,
  };
}

export function hasFeature(entitlement: VerifiedEntitlement | null, key: string): boolean {
  return entitlement?.features.includes(key) ?? false;
}
