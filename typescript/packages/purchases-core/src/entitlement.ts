/** Entitlement the product backend verified from the hub ticket. */
export interface VerifiedEntitlement {
  readonly suiteSubject: string;
  readonly tier: string;
  readonly features: readonly string[];
  readonly revision: number;
}

export interface CreditBalanceSnapshot {
  readonly suiteSubject: string;
  readonly available: number;
  readonly revision: number;
}

/** Product backend calls. The backend refreshes the hub ticket with its service credentials. */
export interface EntitlementBackend {
  refreshEntitlement(): Promise<VerifiedEntitlement | null>;
  creditBalance?(): Promise<CreditBalanceSnapshot | null>;
}

export interface TicketClaimsLike {
  readonly sub: string;
  readonly tier: string;
  readonly features: readonly string[];
  readonly rev: number;
}

export function entitlementFromTicket(claims: TicketClaimsLike): VerifiedEntitlement {
  return {
    suiteSubject: claims.sub,
    tier: claims.tier,
    features: claims.features,
    revision: claims.rev,
  };
}

export function hasFeature(entitlement: VerifiedEntitlement | null, key: string): boolean {
  return entitlement?.features.includes(key) ?? false;
}
