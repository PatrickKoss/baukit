import {
  isNonNegativeSafeInteger,
  isRecord,
  isString,
  isStringArray,
  nullable,
} from '@baukit/type-guards';
import type {
  BillingChannel,
  BillingClaim,
  CreditBalanceSnapshot,
  EntitlementBackend,
  EntitlementOrganization,
  VerifiedEntitlement,
} from './entitlement.js';

/** Error codes of Baukit's account plan routes. */
export const ACCOUNT_PLAN_ERROR_CODES = [
  'suite_identity_missing',
  'entitlements_unavailable',
  'credits_unavailable',
  'feature_not_entitled',
  'organization_not_accessible',
  'organization_id_invalid',
] as const;

/** The request header that names the organization a request acts for. */
export const ORGANIZATION_HEADER = 'x-organization-id';
export type AccountPlanErrorCode = (typeof ACCOUNT_PLAN_ERROR_CODES)[number];

export class AccountPlanError extends Error {
  constructor(
    readonly status: number,
    readonly code: string,
    readonly retryAfterSeconds: number | null = null,
  ) {
    super(code);
    this.name = 'AccountPlanError';
  }
}

export interface AccountPlanBackendOptions {
  /** The product API prefix that nests the plan router, for example `https://api.example/api/v1`. */
  readonly baseUrl: string;
  readonly fetch: (
    input: string,
    init: { readonly headers: Record<string, string> },
  ) => Promise<Response>;
  /** Called per request, for example to add the bearer token. */
  readonly headers?: () => Promise<Record<string, string>> | Record<string, string>;
}

const billingChannels: readonly BillingChannel[] = [
  'web',
  'app_store',
  'play_store',
  'family',
  'organization',
  'promo',
];

const isNullableString = nullable(isString);
const isNullableCount = nullable(isNonNegativeSafeInteger);

function isBillingChannel(value: unknown): value is BillingChannel {
  return billingChannels.some((channel) => channel === value);
}

function invalidEntitlement(): never {
  throw new TypeError('The API returned an invalid entitlement.');
}

function invalidBalance(): never {
  throw new TypeError('The API returned an invalid credit balance.');
}

function parseBilling(value: unknown): BillingClaim | null {
  if (value === null) return null;
  if (!isRecord(value) || !isBillingChannel(value['channel'])) return invalidEntitlement();
  const { soldBy, manageUrl } = value;
  if (![soldBy, manageUrl].every((field) => field === undefined || isString(field)))
    return invalidEntitlement();
  return {
    channel: value['channel'],
    ...(isString(soldBy) ? { soldBy } : {}),
    ...(isString(manageUrl) ? { manageUrl } : {}),
  };
}

function parseOrganization(value: unknown): EntitlementOrganization | null {
  if (value === null) return null;
  if (!isRecord(value) || !isString(value['organizationId']) || !isNullableString(value['name']))
    return invalidEntitlement();
  return { organizationId: value['organizationId'], name: value['name'] };
}

function parseEntitlement(value: unknown): VerifiedEntitlement {
  if (!isRecord(value)) return invalidEntitlement();
  const { mode, suiteSubject, tier, features, revision, plansUrl } = value;
  if (
    (mode !== 'hub' && mode !== 'standalone') ||
    !isString(suiteSubject) ||
    !isString(tier) ||
    !isStringArray(features) ||
    !isNonNegativeSafeInteger(revision) ||
    !isNullableString(plansUrl)
  ) {
    return invalidEntitlement();
  }
  const entitlement: VerifiedEntitlement = {
    mode,
    suiteSubject,
    tier,
    features,
    revision,
    plansUrl,
    billing: parseBilling(value['billing']),
  };
  if (value['organization'] === undefined) return entitlement;
  return { ...entitlement, organization: parseOrganization(value['organization']) };
}

function parsePack(value: unknown): CreditBalanceSnapshot['packs'][number] {
  if (
    !isRecord(value) ||
    !isNonNegativeSafeInteger(value['available']) ||
    !isNonNegativeSafeInteger(value['expiresAt'])
  )
    return invalidBalance();
  return { available: value['available'], expiresAt: value['expiresAt'] };
}

function parseGrant(value: unknown): CreditBalanceSnapshot['grant'] {
  if (
    !isRecord(value) ||
    !isNonNegativeSafeInteger(value['available']) ||
    !isNullableCount(value['expiresAt'])
  )
    return invalidBalance();
  return { available: value['available'], expiresAt: value['expiresAt'] };
}

function parseBalance(value: unknown): CreditBalanceSnapshot {
  if (!isRecord(value)) return invalidBalance();
  const { suiteSubject, wallet, revision, total, packs } = value;
  if (
    !isString(suiteSubject) ||
    !isString(wallet) ||
    !isNonNegativeSafeInteger(revision) ||
    !isNonNegativeSafeInteger(total) ||
    !Array.isArray(packs)
  ) {
    return invalidBalance();
  }
  return {
    suiteSubject,
    wallet,
    revision,
    total,
    grant: parseGrant(value['grant']),
    packs: packs.map(parsePack),
  };
}

async function failure(response: Response): Promise<AccountPlanError> {
  const retry = Number(response.headers.get('retry-after'));
  let code = 'internal_error';
  try {
    const body: unknown = await response.json();
    const error = isRecord(body) ? body['error'] : undefined;
    if (isRecord(error) && isString(error['code'])) code = error['code'];
  } catch {
    code = 'internal_error';
  }
  return new AccountPlanError(
    response.status,
    code,
    Number.isSafeInteger(retry) && retry > 0 ? retry : null,
  );
}

/**
 * Reads Baukit's `GET /me/entitlement` and `GET /me/credits`. `suite_identity_missing` resolves
 * to `null`. `organizationEntitlement` rejects a non-member with `organization_not_accessible`.
 */
export function createAccountPlanBackend(options: AccountPlanBackendOptions): EntitlementBackend {
  const base = options.baseUrl.replace(/\/+$/u, '');
  async function read<T>(
    path: string,
    parse: (value: unknown) => T,
    extra: Record<string, string> = {},
  ): Promise<T | null> {
    const headers = { accept: 'application/json', ...(await options.headers?.()), ...extra };
    const response = await options.fetch(`${base}${path}`, { headers });
    if (!response.ok) {
      const error = await failure(response);
      if (error.code === 'suite_identity_missing') return null;
      throw error;
    }
    return parse(await response.json());
  }
  return {
    refreshEntitlement: () => read('/me/entitlement', parseEntitlement),
    creditBalance: () => read('/me/credits', parseBalance),
    organizationEntitlement: (organizationId) =>
      read('/me/entitlement', parseEntitlement, { [ORGANIZATION_HEADER]: organizationId }),
  };
}
