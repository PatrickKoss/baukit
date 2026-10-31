/** Platform event types without the hub's configured prefix, in catalog order. */
export const PLATFORM_EVENT_TYPES = [
  'entitlements.changed',
  'account.updated',
  'account.erasure_requested',
  'preferences.changed',
  'onboarding.shared',
  'family.membership_changed',
  'organization.membership_changed',
  'seat.changed',
  'organization.updated',
  'credits.changed',
] as const;
export type PlatformEventType = (typeof PLATFORM_EVENT_TYPES)[number];

export interface SubjectRevision {
  readonly suiteSubject: string;
  readonly revision: number;
}
export interface PlatformEventPayloads {
  readonly 'entitlements.changed': SubjectRevision & { readonly product: string };
  readonly 'account.updated': SubjectRevision;
  readonly 'account.erasure_requested': {
    readonly suiteSubject: string;
    readonly erasureId: string;
  };
  readonly 'preferences.changed': SubjectRevision;
  readonly 'onboarding.shared': SubjectRevision & { readonly profileVersion: number };
  readonly 'family.membership_changed': SubjectRevision & {
    readonly familyId: string;
    readonly status: 'active' | 'removed' | 'left';
    readonly familyRole: 'organizer' | 'member';
  };
  readonly 'organization.membership_changed': {
    readonly organizationId: string;
    readonly suiteSubject: string;
    readonly status: 'active' | 'suspended' | 'removed';
    readonly accountRole: 'owner' | 'admin' | 'billing' | 'member';
  };
  readonly 'seat.changed': {
    readonly organizationId: string;
    readonly product: string;
    readonly suiteSubject: string;
    readonly assigned: boolean;
  };
  /** The envelope `userId` is the organization ID. */
  readonly 'organization.updated': {
    readonly organizationId: string;
    readonly revision: number;
  };
  readonly 'credits.changed': SubjectRevision & { readonly wallet: string };
}

export type CreditReservationStatus = 'pending' | 'settled' | 'released' | 'expired';
export interface CreditReservation {
  readonly id: string;
  readonly wallet: string;
  readonly amount: number;
  readonly charged: number | null;
  readonly status: CreditReservationStatus;
  /** Unix seconds. */
  readonly expiresAt: number;
}
export interface CreditPool {
  /** `family` or `organization`. */
  readonly ownerKind: string;
  readonly available: number;
}
export interface CreditGrant {
  readonly granted: number;
  readonly used: number;
  readonly available: number;
}
export interface CreditPack {
  readonly remaining: number;
  /** Unix seconds. */
  readonly expiresAt: number;
}
export interface CreditBalance {
  readonly suiteSubject: string;
  readonly wallet: string;
  readonly available: number;
  /** Counts `credits.changed` events for this wallet. */
  readonly revision: number;
  readonly pools: readonly CreditPool[];
  readonly grant: CreditGrant;
  readonly packs: readonly CreditPack[];
}

export type OrganizationStatus = 'active' | 'suspended';
export interface OrganizationSummary {
  readonly organizationId: string;
  /** Display name. Absent from a hub before Schlüsselbund C16. */
  readonly name?: string;
  readonly status: OrganizationStatus;
  readonly productId: string;
  readonly tier: string;
  readonly features: readonly string[];
  /** `null` for a per-organization plan without a seat limit. */
  readonly paidQuantity: number | null;
  readonly assignedSeats: number;
  /** Matches the latest `organization.updated` event, or 0 before the first. */
  readonly revision: number;
}

export type OrganizationRole = 'owner' | 'admin' | 'billing' | 'member';
/** An organization in which a subject can use the calling product. */
export interface SubjectOrganization {
  readonly organizationId: string;
  readonly accountRole: OrganizationRole;
}
/** `GET /platform/v1/accounts/{suiteSubject}/organizations`, sorted by organization ID. */
export interface SubjectOrganizationPage {
  readonly items: readonly SubjectOrganization[];
  /** `null` on the last page. */
  readonly nextCursor: string | null;
}

/** Hub error codes the Rust credits port maps to typed errors. */
export const PLATFORM_CREDIT_ERROR_CODES = [
  'account_inactive',
  'credits_amount_invalid',
  'credits_idempotency_conflict',
  'credits_idempotency_key_invalid',
  'credits_insufficient',
  'credits_reservation_expired',
  'credits_reservation_not_found',
  'credits_wallet_unknown',
  'platform_credits_unavailable',
  'platform_subject_not_found',
] as const;
export type PlatformCreditErrorCode = (typeof PLATFORM_CREDIT_ERROR_CODES)[number];
