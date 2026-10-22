import { ed25519 } from '@noble/curves/ed25519';

export type TicketErrorCode = 'ticket_invalid' | 'ticket_expired' | 'ticket_wrong_audience';

export class TicketVerificationError extends Error {
  constructor(readonly code: TicketErrorCode) {
    super(code);
    this.name = 'TicketVerificationError';
  }
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
  if (typeof value !== 'object' || value === null || Array.isArray(value)) invalid();
  return value as Record<string, unknown>;
}
function nonempty(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}
function integer(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value);
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
function claims(value: Record<string, unknown>, options: TicketVerificationOptions): TicketClaims {
  const allowed = ['iss', 'sub', 'aud', 'tier', 'features', 'rev', 'iat', 'exp', 'grace'];
  if (
    Object.keys(value).some((key) => !allowed.includes(key)) ||
    !nonempty(value['iss']) ||
    !nonempty(value['sub']) ||
    !nonempty(value['aud']) ||
    !nonempty(value['tier']) ||
    !Array.isArray(value['features']) ||
    !value['features'].every((feature: unknown) => typeof feature === 'string') ||
    !integer(value['rev']) ||
    value['rev'] < 0 ||
    !integer(value['iat']) ||
    value['iat'] < 0 ||
    !integer(value['exp']) ||
    ('grace' in value && !integer(value['grace']))
  )
    invalid();
  const result = value as unknown as TicketClaims;
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
    if (
      ticket.length > maxTicketBytes ||
      !integer(options.now) ||
      !options.issuer ||
      !options.audience
    )
      invalid();
    const [headerPart, payloadPart, signaturePart, extra] = ticket.split('.');
    if (!headerPart || !payloadPart || !signaturePart || extra !== undefined) invalid();
    const header = json(headerPart);
    if (
      header['alg'] !== 'EdDSA' ||
      !nonempty(header['kid']) ||
      header['kid'].length > maxKidLength ||
      ('typ' in header && header['typ'] !== 'JWT') ||
      Object.keys(header).some((key) => !['alg', 'kid', 'typ'].includes(key))
    )
      invalid();
    if (
      jwks.keys.length > maxJwksKeys ||
      new Set(jwks.keys.map((key) => key.kid)).size !== jwks.keys.length
    )
      invalid();
    const key = jwks.keys.find((key) => key.kid === header['kid']);
    if (
      key?.kty !== 'OKP' ||
      key.crv !== 'Ed25519' ||
      (key.alg !== undefined && key.alg !== 'EdDSA') ||
      (key.use !== undefined && key.use !== 'sig') ||
      key.key_ops?.includes('verify') === false
    )
      invalid();
    const input = Uint8Array.from(`${headerPart}.${payloadPart}`, (char) => char.charCodeAt(0));
    if (!ed25519.verify(decode(signaturePart), input, decode(key.x), { zip215: false })) invalid();
    return claims(json(payloadPart), options);
  } catch (error) {
    if (error instanceof TicketVerificationError) throw error;
    return invalid();
  }
}
