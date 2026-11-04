import type {
  CreditBalanceSnapshot,
  EntitlementBackend,
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

type Json = Record<string, unknown>;

function record(value: unknown): value is Json {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function count(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
}
function nullable(value: unknown, check: (value: unknown) => boolean): boolean {
  return value === null || check(value);
}
function text(value: unknown): value is string {
  return typeof value === 'string';
}

function parseEntitlement(value: unknown): VerifiedEntitlement {
  if (
    !record(value) ||
    (value['mode'] !== 'hub' && value['mode'] !== 'standalone') ||
    !text(value['suiteSubject']) ||
    !text(value['tier']) ||
    !Array.isArray(value['features']) ||
    !value['features'].every(text) ||
    !count(value['revision']) ||
    !nullable(value['plansUrl'], text) ||
    !nullable(value['billing'], record) ||
    !(value['organization'] === undefined || nullable(value['organization'], organization))
  ) {
    throw new TypeError('The API returned an invalid entitlement.');
  }
  return value as unknown as VerifiedEntitlement;
}

function organization(value: unknown): boolean {
  return record(value) && text(value['organizationId']) && nullable(value['name'], text);
}

function parseBalance(value: unknown): CreditBalanceSnapshot {
  const grant = record(value) ? value['grant'] : undefined;
  const packs = record(value) ? value['packs'] : undefined;
  if (
    !record(value) ||
    !text(value['suiteSubject']) ||
    !text(value['wallet']) ||
    !count(value['revision']) ||
    !count(value['total']) ||
    !record(grant) ||
    !count(grant['available']) ||
    !nullable(grant['expiresAt'], count) ||
    !Array.isArray(packs) ||
    !packs.every((pack) => record(pack) && count(pack['available']) && count(pack['expiresAt']))
  ) {
    throw new TypeError('The API returned an invalid credit balance.');
  }
  return value as unknown as CreditBalanceSnapshot;
}

async function failure(response: Response): Promise<AccountPlanError> {
  const retry = Number(response.headers.get('retry-after'));
  let code = 'internal_error';
  try {
    const body: unknown = await response.json();
    const error = record(body) ? body['error'] : undefined;
    if (record(error) && text(error['code'])) code = error['code'];
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
