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
  EntitlementOrganization,
  VerifiedEntitlement,
} from './entitlement.js';

const billingChannels: readonly BillingChannel[] = [
  'web',
  'app_store',
  'play_store',
  'family',
  'organization',
  'promo',
];

const isNullableString = nullable(isString);

function isBillingChannel(value: unknown): value is BillingChannel {
  return billingChannels.some((channel) => channel === value);
}

function invalidEntitlement(): never {
  throw new TypeError('The API returned an invalid entitlement.');
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

export function parseEntitlement(value: unknown): VerifiedEntitlement {
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
