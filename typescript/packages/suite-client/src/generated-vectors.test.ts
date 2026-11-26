import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { verifyTicket, type TicketJwks } from './entitlements.js';
import { suiteReturnValidator } from './urls.js';

interface Generated {
  tickets: {
    issuer: string;
    audience: string;
    now: number;
    jwks: TicketJwks;
    cases: { name: string; ticket: string; valid: boolean }[];
  };
  returnUrls: {
    origin: string;
    scheme: string;
    cases: { url: string; allowDevScheme: boolean; valid: boolean }[];
  };
}

const generated = JSON.parse(
  readFileSync(new URL('../../../../fixtures/suite/generated-v1.json', import.meta.url), 'utf8'),
) as Generated;

function accepts(verify: () => unknown): boolean {
  try {
    verify();
    return true;
  } catch {
    return false;
  }
}

describe('D-03 tickets agree with the Rust decoder', () => {
  const { issuer, audience, now, jwks } = generated.tickets;
  it.each(generated.tickets.cases)('$name', ({ ticket, valid }) => {
    expect(accepts(() => verifyTicket(ticket, jwks, { issuer, audience, now }))).toBe(valid);
  });
});

describe('D-05 return URLs agree with validate_return_url', () => {
  const { origin, scheme } = generated.returnUrls;
  const web = suiteReturnValidator({ platform: 'web', origin });
  const native = suiteReturnValidator({ platform: 'native', origin: `${scheme}://suite` });
  const dev = suiteReturnValidator({ platform: 'native', origin: `${scheme}-dev://suite` });
  it.each(generated.returnUrls.cases)(
    '$url dev=$allowDevScheme',
    ({ url, allowDevScheme, valid }) => {
      const client = web(url) || native(url) || (allowDevScheme && dev(url));
      expect(client).toBe(valid);
    },
  );
});
