import {
  isBoolean,
  isNonNegativeSafeInteger,
  isRecord,
  isString,
  isStringArray,
  nullable,
} from '@baukit/type-guards';
import type {
  AuthorizationPreview,
  DeliveryHealth,
  RewardMode,
  SuiteDelivery,
  SuiteLink,
  SuitePeer,
} from './api.js';

export class SuiteResponseError extends Error {
  public readonly code = 'suite_unavailable';
  public constructor(public readonly endpoint: string) {
    super(`suite response for ${endpoint} has an unexpected shape`);
  }
}

type Guard<T> = (value: unknown) => value is T;

const REWARD_MODES: readonly string[] = ['native', 'source_xp', 'off'];
const DELIVERY_HEALTHS: readonly string[] = ['healthy', 'degraded', 'needs_attention', 'disabled'];
const LINK_ROLES: readonly string[] = ['initiator', 'authorizer'];
const LINK_STATUSES: readonly string[] = ['active', 'needs_attention', 'revoked'];
const DELIVERY_STATUSES: readonly string[] = [
  'pending',
  'running',
  'succeeded',
  'failed',
  'cancelled',
];
const isNullableString = nullable(isString);

function isOneOf<T extends string>(allowed: readonly string[]): Guard<T> {
  return (value): value is T => typeof value === 'string' && allowed.includes(value);
}
const isRewardMode = isOneOf<RewardMode>(REWARD_MODES);
const isDeliveryHealth = isOneOf<DeliveryHealth>(DELIVERY_HEALTHS);
const isLinkRole = isOneOf<SuiteLink['role']>(LINK_ROLES);
const isLinkStatus = isOneOf<SuiteLink['status']>(LINK_STATUSES);
const isDeliveryStatus = isOneOf<SuiteDelivery['status']>(DELIVERY_STATUSES);
const isRewardModeArray = arrayOf(isRewardMode);

function arrayOf<T>(guard: Guard<T>): Guard<T[]> {
  return (value): value is T[] => Array.isArray(value) && value.every(guard);
}

type Shape<T> = { readonly [K in keyof T]-?: Guard<T[K]> };

function shaped<T extends object>(shape: Shape<T>): Guard<T> {
  const fields = Object.entries<Guard<unknown>>(shape);
  return (value): value is T =>
    isRecord(value) && fields.every(([key, guard]) => guard(value[key]));
}

export interface SuiteLinkStart {
  readonly requestId: string;
  readonly authorizeUrl: string;
}
export interface SuiteRedirect {
  readonly redirectUrl: string;
}

export const isSuiteLink = shaped<SuiteLink>({
  id: isString,
  peerApp: isString,
  role: isLinkRole,
  remoteLinkId: isString,
  remoteDisplayName: isNullableString,
  status: isLinkStatus,
  sends: isStringArray,
  receives: isStringArray,
  shareXp: isBoolean,
  rewardMode: isRewardMode,
  deliveryHealth: isDeliveryHealth,
  consecutiveFailures: isNonNegativeSafeInteger,
  lastDeliveryAt: isNullableString,
  lastFailureAt: isNullableString,
  lastFailureCode: isNullableString,
  lastReceivedAt: isNullableString,
  replayEarliest: isString,
  createdAt: isString,
  updatedAt: isString,
});
export const isSuitePeer = shaped<SuitePeer>({
  id: isString,
  displayName: isString,
  scheme: isString,
  webUrl: isString,
  shareXpAvailable: isBoolean,
  sends: isStringArray,
  receives: isStringArray,
  rewardModes: isRewardModeArray,
  link: isNullableString,
});
export const isAuthorizationPreview = shaped<AuthorizationPreview>({
  peer: isString,
  authorizerSends: isStringArray,
  initiatorSends: isStringArray,
  existingLink: isBoolean,
  autoApprove: isBoolean,
  hintMismatch: isBoolean,
});
export const isSuiteDelivery = shaped<SuiteDelivery>({
  jobId: isString,
  linkId: isString,
  eventType: isNullableString,
  status: isDeliveryStatus,
  attemptCount: isNonNegativeSafeInteger,
  lastFailureCode: isNullableString,
  createdAt: isString,
  updatedAt: isString,
});
export const isSuiteLinkStart = shaped<SuiteLinkStart>({
  requestId: isString,
  authorizeUrl: isString,
});
export const isSuiteRedirect = shaped<SuiteRedirect>({ redirectUrl: isString });

export const isSuitePeers = arrayOf(isSuitePeer);
export const isSuiteLinks = arrayOf(isSuiteLink);
export const isSuiteDeliveries = arrayOf(isSuiteDelivery);

export function parseSuiteResponse<T>(endpoint: string, value: unknown, guard: Guard<T>): T {
  if (!guard(value)) throw new SuiteResponseError(endpoint);
  return value;
}
