import { ed25519 } from '@noble/curves/ed25519';
import {
  isNonEmptyString,
  isNonNegativeSafeInteger,
  isRecord,
  isSafeInteger,
  isStringArray,
} from '@baukit/type-guards';

export type TicketErrorCode = 'ticket_invalid' | 'ticket_expired' | 'ticket_wrong_audience';

export class TicketVerificationError extends Error {
  constructor(readonly code: TicketErrorCode) {
    super(code);
    this.name = 'TicketVerificationError';
  }
}

export type BillingChannel =
  'web' | 'app_store' | 'play_store' | 'family' | 'organization' | 'promo';

/** Where the hub says paid access is managed. `soldBy` only for stores, `manageUrl` only for web. */
export interface BillingClaim {
  readonly channel: BillingChannel;
  readonly soldBy?: string;
  readonly manageUrl?: string;
}

export interface TicketClaims {
  readonly iss: string;
  readonly sub: string;
  readonly aud: string;
  readonly tier: string;
  readonly features: readonly string[];
  readonly rev: number;
  readonly iat: number;
  readonly exp: number;
  readonly grace?: number;
  readonly billing?: BillingClaim;
}

export interface TicketJwk {
  readonly kid: string;
  readonly kty: string;
  readonly crv: string;
  readonly x: string;
  readonly alg?: string;
  readonly use?: string;
  readonly key_ops?: readonly string[];
}
export interface TicketJwks {
  readonly keys: readonly TicketJwk[];
}
export interface TicketVerificationOptions {
  readonly issuer: string;
  readonly audience: string;
  /** Unix seconds. Expiry is exclusive; only iat has a 60-second allowance. */
  readonly now: number;
}

const alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_';
const maxTicketBytes = 32 * 1024;
const maxLifetimeSeconds = 7 * 24 * 60 * 60;
const iatLeewaySeconds = 60;
const maxKidLength = 256;
const maxJwksKeys = 64;

function invalid(): never {
  throw new TicketVerificationError('ticket_invalid');
}
function record(value: unknown): Record<string, unknown> {
  if (!isRecord(value)) invalid();
  return value;
}
function decode(value: string): Uint8Array {
  if (!/^[A-Za-z0-9_-]+$/u.test(value) || value.length % 4 === 1) invalid();
  const bytes = new Uint8Array(Math.floor((value.length * 6) / 8));
  let bits = 0;
  let buffer = 0;
  let offset = 0;
  for (const char of value) {
    buffer = (buffer << 6) | alphabet.indexOf(char);
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      bytes[offset++] = (buffer >> bits) & 255;
    }
  }
  if ((buffer & ((1 << bits) - 1)) !== 0) invalid();
  return bytes;
}
function json(value: string): Record<string, unknown> {
  // Hermes does not require TextDecoder, atob or Buffer for this path.
  const escaped = Array.from(
    decode(value),
    (byte) => `%${byte.toString(16).padStart(2, '0')}`,
  ).join('');
  return record(JSON.parse(decodeURIComponent(escaped)) as unknown);
}
const billingChannels: readonly BillingChannel[] = [
  'web',
  'app_store',
  'play_store',
  'family',
  'organization',
  'promo',
];
function isBillingChannel(value: unknown): value is BillingChannel {
  return billingChannels.some((channel) => channel === value);
}
// Hermes has no complete URL class, so the check stays a pattern.
const httpUrlPattern = /^https?:\/\/[^\s/?#@]+(?:[/?#]\S*)?$/u;
function httpUrl(value: string): boolean {
  return httpUrlPattern.test(value);
}
function billingClaim(value: unknown): BillingClaim | undefined {
  if (!isRecord(value)) return undefined;
  const channel = value['channel'];
  if (
    Object.keys(value).some((key) => !['channel', 'soldBy', 'manageUrl'].includes(key)) ||
    !isBillingChannel(channel)
  )
    return undefined;
  const store = channel === 'app_store' || channel === 'play_store';
  const soldBy = value['soldBy'];
  if ('soldBy' in value && !(store && isNonEmptyString(soldBy))) return undefined;
  const manageUrl = value['manageUrl'];
  if (
    'manageUrl' in value &&
    !(channel === 'web' && typeof manageUrl === 'string' && httpUrl(manageUrl))
  )
    return undefined;
  return {
    channel,
    ...(typeof soldBy === 'string' ? { soldBy } : {}),
    ...(typeof manageUrl === 'string' ? { manageUrl } : {}),
  };
}
const allowedClaims = [
  'iss',
  'sub',
  'aud',
  'tier',
  'features',
  'rev',
  'iat',
  'exp',
  'grace',
  'billing',
];
function optionalClaims(value: Record<string, unknown>): Pick<TicketClaims, 'grace' | 'billing'> {
  const grace = value['grace'];
  if ('grace' in value && !isSafeInteger(grace)) invalid();
  const billing = 'billing' in value ? billingClaim(value['billing']) : undefined;
  if ('billing' in value && billing === undefined) invalid();
  return {
    ...(isSafeInteger(grace) ? { grace } : {}),
    ...(billing === undefined ? {} : { billing }),
  };
}
function parseClaims(value: Record<string, unknown>): TicketClaims {
  const { iss, sub, aud, tier, features, rev, iat, exp } = value;
  if (
    Object.keys(value).some((key) => !allowedClaims.includes(key)) ||
    !isNonEmptyString(iss) ||
    !isNonEmptyString(sub) ||
    !isNonEmptyString(aud) ||
    !isNonEmptyString(tier) ||
    !isStringArray(features) ||
    !isNonNegativeSafeInteger(rev) ||
    !isNonNegativeSafeInteger(iat) ||
    !isSafeInteger(exp)
  )
    invalid();
  return { iss, sub, aud, tier, features, rev, iat, exp, ...optionalClaims(value) };
}
function claims(value: Record<string, unknown>, options: TicketVerificationOptions): TicketClaims {
  const result = parseClaims(value);
  if (
    result.iss !== options.issuer ||
    result.iat > options.now + iatLeewaySeconds ||
    result.exp <= result.iat ||
    result.exp - result.iat > maxLifetimeSeconds
  )
    invalid();
  if (result.exp <= options.now) throw new TicketVerificationError('ticket_expired');
  if (result.aud !== options.audience) throw new TicketVerificationError('ticket_wrong_audience');
  return result;
}

interface TicketParts {
  readonly header: string;
  readonly payload: string;
  readonly signature: string;
}
function ticketParts(ticket: string, options: TicketVerificationOptions): TicketParts {
  if (
    ticket.length > maxTicketBytes ||
    !isSafeInteger(options.now) ||
    options.issuer === '' ||
    options.audience === ''
  )
    invalid();
  const [header, payload, signature, extra] = ticket.split('.');
  if (
    !isNonEmptyString(header) ||
    !isNonEmptyString(payload) ||
    !isNonEmptyString(signature) ||
    extra !== undefined
  )
    invalid();
  return { header, payload, signature };
}
function headerKid(header: Record<string, unknown>): string {
  const kid = header['kid'];
  if (
    header['alg'] !== 'EdDSA' ||
    !isNonEmptyString(kid) ||
    kid.length > maxKidLength ||
    ('typ' in header && header['typ'] !== 'JWT') ||
    Object.keys(header).some((key) => !['alg', 'kid', 'typ'].includes(key))
  )
    invalid();
  return kid;
}
function verificationKey(jwks: TicketJwks, kid: string): TicketJwk {
  if (
    jwks.keys.length > maxJwksKeys ||
    new Set(jwks.keys.map((key) => key.kid)).size !== jwks.keys.length
  )
    invalid();
  const key = jwks.keys.find((candidate) => candidate.kid === kid);
  if (
    key?.kty !== 'OKP' ||
    key.crv !== 'Ed25519' ||
    (key.alg !== undefined && key.alg !== 'EdDSA') ||
    (key.use !== undefined && key.use !== 'sig') ||
    key.key_ops?.includes('verify') === false
  )
    invalid();
  return key;
}

/** Verify offline against trusted, embedded or previously fetched hub public keys.
 * Compare the returned sub with the current suite subject before granting access.
 * Refresh keys on an unknown kid through the product's trusted network adapter.
 */
export function verifyTicket(
  ticket: string,
  jwks: TicketJwks,
  options: TicketVerificationOptions,
): TicketClaims {
  try {
    const parts = ticketParts(ticket, options);
    const key = verificationKey(jwks, headerKid(json(parts.header)));
    const input = Uint8Array.from(`${parts.header}.${parts.payload}`, (char) => char.charCodeAt(0));
    if (!ed25519.verify(decode(parts.signature), input, decode(key.x), { zip215: false }))
      invalid();
    return claims(json(parts.payload), options);
  } catch (error) {
    if (error instanceof TicketVerificationError) throw error;
    return invalid();
  }
}
