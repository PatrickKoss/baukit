import { describe, expect, it } from 'vitest';

import { unverifiedDisplayIdentityHintsFromJwt } from './index.js';

const fallback = { displayName: 'Local account', initials: 'LA' } as const;

function unsignedJwt(payload: unknown): string {
  const json = JSON.stringify(payload);
  const bytes = new TextEncoder().encode(json);
  const binary = Array.from(bytes, (byte) => String.fromCharCode(byte)).join('');
  return `header.${btoa(binary).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/u, '')}.signature`;
}

describe('unverified display identity hints', () => {
  it('prefers name and derives at most two Unicode-aware initials', () => {
    expect(
      unverifiedDisplayIdentityHintsFromJwt(
        unsignedJwt({ name: '  Émilie du Châtelet  ', preferred_username: 'emilie' }),
        fallback,
      ),
    ).toEqual({ displayName: 'Émilie du Châtelet', initials: 'ÉD' });
  });

  it('combines given and family names before username and email', () => {
    expect(
      unverifiedDisplayIdentityHintsFromJwt(
        unsignedJwt({
          given_name: 'Ada',
          family_name: 'Lovelace',
          preferred_username: 'ada',
          email: 'ada@example.test',
        }),
        fallback,
      ),
    ).toEqual({ displayName: 'Ada Lovelace', initials: 'AL' });
    expect(
      unverifiedDisplayIdentityHintsFromJwt(unsignedJwt({ given_name: 'Ada' }), fallback),
    ).toEqual({ displayName: 'Ada', initials: 'A' });
  });

  it('uses username, then email, when name claims are absent', () => {
    expect(
      unverifiedDisplayIdentityHintsFromJwt(
        unsignedJwt({ preferred_username: 'ada', email: 'other@example.test' }),
        fallback,
      ),
    ).toEqual({ displayName: 'ada', initials: 'A' });
    expect(
      unverifiedDisplayIdentityHintsFromJwt(unsignedJwt({ email: 'ada@example.test' }), fallback),
    ).toEqual({ displayName: 'ada@example.test', initials: 'A' });
  });

  it.each([
    'not-a-jwt',
    'header.%%%%.signature',
    'header.e2JhZA.signature',
    unsignedJwt([]),
    unsignedJwt({ name: '   ', preferred_username: 17 }),
    `header.${'a'.repeat(16_385)}.signature`,
  ])('returns product fallback text for malformed or unusable input', (token) => {
    expect(unverifiedDisplayIdentityHintsFromJwt(token, fallback)).toEqual(fallback);
  });

  it('rejects empty product fallback text even when claims are present', () => {
    expect(() =>
      unverifiedDisplayIdentityHintsFromJwt(unsignedJwt({ name: 'Ada' }), {
        displayName: ' ',
        initials: 'L',
      }),
    ).toThrow(RangeError);
  });
});
