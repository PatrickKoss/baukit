import { readFileSync } from 'node:fs';
import { describe, expect, it, vi } from 'vitest';
import { ed25519 } from '@noble/curves/ed25519';
import {
  verifyTicket,
  TicketVerificationError,
  type TicketJwks,
  type TicketClaims,
} from './entitlements.js';

interface Vector {
  name: string;
  ticket: string;
  audience: string;
  now: number;
  expected: string;
  claims?: TicketClaims;
}
interface Fixture {
  issuer: string;
  testSeedHex: string;
  jwks: TicketJwks;
  vectors: Vector[];
}
const fixture = JSON.parse(
  readFileSync(
    new URL('../../../../fixtures/entitlement-tickets/v1/vectors.json', import.meta.url),
    'utf8',
  ),
) as Fixture;

describe('shared ticket vectors', () => {
  it.each(fixture.vectors)('$name', (v) => {
    const run = () =>
      verifyTicket(v.ticket, fixture.jwks, {
        issuer: fixture.issuer,
        audience: v.audience,
        now: v.now,
      });
    if (v.expected === 'ok') expect(run()).toEqual(v.claims);
    else
      expect(run).toThrow(
        new TicketVerificationError(v.expected as TicketVerificationError['code']),
      );
  });
});
it('runs with the globals absent in Hermes', () => {
  for (const name of ['crypto', 'Buffer', 'atob', 'btoa', 'TextEncoder', 'TextDecoder'])
    vi.stubGlobal(name, undefined);
  try {
    const v = fixture.vectors[0];
    if (!v) throw new Error('missing vector');
    expect(
      verifyTicket(v.ticket, fixture.jwks, {
        issuer: fixture.issuer,
        audience: v.audience,
        now: v.now,
      }),
    ).toEqual(v.claims);
  } finally {
    vi.unstubAllGlobals();
  }
});
it('verifies UTF-8 claims without browser decoders', () => {
  const v = fixture.vectors[0];
  if (!v?.claims) throw new Error('missing claims');
  const header = Buffer.from(
    JSON.stringify({ alg: 'EdDSA', kid: fixture.jwks.keys[0]?.kid }),
  ).toString('base64url');
  const expected = { ...v.claims, sub: 'suite|Grüße 🌍' };
  const body = Buffer.from(JSON.stringify(expected)).toString('base64url');
  const input = `${header}.${body}`;
  const ticket = `${input}.${Buffer.from(ed25519.sign(Buffer.from(input), fixture.testSeedHex)).toString('base64url')}`;
  expect(
    verifyTicket(ticket, fixture.jwks, {
      issuer: fixture.issuer,
      audience: v.audience,
      now: v.now,
    }),
  ).toEqual(expected);
});
it('rejects duplicate kids and incompatible key metadata', () => {
  const v = fixture.vectors[0];
  const key = fixture.jwks.keys[0];
  if (!v || !key) throw new Error('missing vector');
  for (const keys of [
    [key, key],
    [{ ...key, use: 'enc' }],
    [{ ...key, alg: 'HS256' }],
    [{ ...key, key_ops: ['sign'] }],
  ]) {
    expect(() =>
      verifyTicket(
        v.ticket,
        { keys },
        { issuer: fixture.issuer, audience: v.audience, now: v.now },
      ),
    ).toThrow('ticket_invalid');
  }
});
it('loads the published subpath', async () => {
  const exported = await import('@baukit/suite-client/entitlements');
  expect(typeof exported.verifyTicket).toBe('function');
});

it('rejects an unsigned ticket with alg none', () => {
  const v = fixture.vectors[0];
  if (!v) throw new Error('missing vector');
  const header = Buffer.from(
    JSON.stringify({ alg: 'none', kid: fixture.jwks.keys[0]?.kid }),
  ).toString('base64url');
  const ticket = [header, ...v.ticket.split('.').slice(1)].join('.');
  expect(() =>
    verifyTicket(ticket, fixture.jwks, {
      issuer: fixture.issuer,
      audience: v.audience,
      now: v.now,
    }),
  ).toThrow('ticket_invalid');
});
