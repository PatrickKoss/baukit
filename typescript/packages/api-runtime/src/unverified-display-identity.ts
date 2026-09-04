const MAX_JWT_PAYLOAD_CHARACTERS = 16_384;

/** Unverified JWT text suitable only for display. */
export interface UnverifiedDisplayIdentityHints {
  readonly displayName: string;
  readonly initials: string;
}

/** Product copy used when a JWT has no usable display claims. */
export interface UnverifiedDisplayIdentityFallback {
  readonly displayName: string;
  readonly initials: string;
}

/**
 * Decodes unverified JWT claims and chooses display-only identity hints.
 *
 * Never use this result for authorization, storage partitioning, analytics
 * identity, or any other security decision. Use a server-validated subject for
 * those decisions.
 */
export function unverifiedDisplayIdentityHintsFromJwt(
  token: string,
  fallback: UnverifiedDisplayIdentityFallback,
): UnverifiedDisplayIdentityHints {
  const safeFallback = requiredFallback(fallback);
  const payload = decodeUnverifiedJwtPayload(token);
  if (payload === undefined) return safeFallback;

  const givenName = textClaim(payload, 'given_name');
  const familyName = textClaim(payload, 'family_name');
  const combinedName = [givenName, familyName].filter((value) => value !== undefined).join(' ');
  const displayName =
    textClaim(payload, 'name') ??
    (combinedName === '' ? undefined : combinedName) ??
    textClaim(payload, 'preferred_username') ??
    textClaim(payload, 'email');
  if (displayName === undefined) return safeFallback;

  const initials = displayName
    .split(/\s+/u)
    .filter((part) => part !== '')
    .slice(0, 2)
    .map((part) => Array.from(part)[0]?.toUpperCase() ?? '')
    .join('');

  return { displayName, initials: initials === '' ? safeFallback.initials : initials };
}

function requiredFallback(
  fallback: UnverifiedDisplayIdentityFallback,
): UnverifiedDisplayIdentityFallback {
  const displayName = fallback.displayName.trim();
  const initials = fallback.initials.trim();
  if (displayName === '' || initials === '') {
    throw new RangeError('Display identity fallback text must not be empty.');
  }
  return { displayName, initials };
}

function decodeUnverifiedJwtPayload(token: string): Readonly<Record<string, unknown>> | undefined {
  const segments = token.split('.');
  const encoded = segments.length === 3 ? segments[1] : undefined;
  if (
    encoded === undefined ||
    encoded === '' ||
    encoded.length > MAX_JWT_PAYLOAD_CHARACTERS ||
    !/^[A-Za-z0-9_-]+$/u.test(encoded)
  ) {
    return undefined;
  }

  try {
    const normalized = encoded.replaceAll('-', '+').replaceAll('_', '/');
    const binary = atob(`${normalized}${'='.repeat((4 - (normalized.length % 4)) % 4)}`);
    const escapedBytes = Array.from(
      binary,
      (character) => `%${character.charCodeAt(0).toString(16).padStart(2, '0')}`,
    ).join('');
    const value: unknown = JSON.parse(decodeURIComponent(escapedBytes));
    return isPlainRecord(value) ? value : undefined;
  } catch {
    return undefined;
  }
}

function isPlainRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) return false;
  const prototype = Object.getPrototypeOf(value) as unknown;
  return prototype === Object.prototype || prototype === null;
}

function textClaim(payload: Readonly<Record<string, unknown>>, claim: string): string | undefined {
  const value = payload[claim];
  return typeof value === 'string' && value.trim() !== '' ? value.trim() : undefined;
}
