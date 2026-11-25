import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import fc from 'fast-check';
import { ed25519 } from '@noble/curves/ed25519';
import { verifyTicket, type TicketClaims, type TicketJwks } from './entitlements.js';

interface Fixture {
  issuer: string;
  testSeedHex: string;
  jwks: TicketJwks;
  vectors: { audience: string; now: number; claims?: TicketClaims }[];
}
const fixture = JSON.parse(
  readFileSync(
    new URL('../../../../fixtures/entitlement-tickets/v1/vectors.json', import.meta.url),
    'utf8',
  ),
) as Fixture;
const vector = fixture.vectors.find((v) => v.claims !== undefined);
if (vector?.claims === undefined) throw new Error('missing claims vector');
const { audience, now, claims } = vector;
const kid = fixture.jwks.keys[0]?.kid ?? '';

function signed(headerText: string, bodyText: string): string {
  const input = `${Buffer.from(headerText).toString('base64url')}.${Buffer.from(bodyText).toString('base64url')}`;
  const signature = ed25519.sign(Buffer.from(input), fixture.testSeedHex);
  return `${input}.${Buffer.from(signature).toString('base64url')}`;
}
function verify(headerText: string, bodyText: string): unknown {
  return verifyTicket(signed(headerText, bodyText), fixture.jwks, {
    issuer: fixture.issuer,
    audience,
    now,
  });
}
const header = JSON.stringify({ alg: 'EdDSA', kid });
const body = JSON.stringify(claims);

describe('ticket JSON matches the Rust decoder', () => {
  it('accepts the canonical encoding', () => {
    expect(verify(header, body)).toEqual(claims);
  });

  it('rejects a duplicated claim, whichever value comes last', () => {
    fc.assert(
      fc.property(fc.constantFrom(...Object.keys(claims)), fc.jsonValue(), (key, value) => {
        const duplicated = `${body.slice(0, -1)},${JSON.stringify(key)}:${JSON.stringify(value)}}`;
        expect(() => verify(header, duplicated)).toThrow('ticket_invalid');
      }),
    );
  });

  it('rejects a duplicated header field', () => {
    expect(() => verify(`{"alg":"EdDSA","kid":"${kid}","alg":"EdDSA"}`, body)).toThrow(
      'ticket_invalid',
    );
  });

  it('rejects integers written as floats or exponents', () => {
    fc.assert(
      fc.property(fc.constantFrom('.0', 'e0', 'E0', '.000'), (suffix) => {
        const text = body.replace(
          `"exp":${String(claims.exp)}`,
          `"exp":${String(claims.exp)}${suffix}`,
        );
        expect(() => verify(header, text)).toThrow('ticket_invalid');
      }),
    );
  });

  it('keeps accepting strings that merely contain dots, braces or quotes', () => {
    fc.assert(
      fc.property(fc.string(), (text) => {
        const tagged = JSON.stringify({ ...claims, note: text });
        expect(verify(header, tagged)).toMatchObject({ sub: claims.sub });
      }),
      { numRuns: 200 },
    );
  });
});
