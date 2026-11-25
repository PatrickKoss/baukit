import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import fc from 'fast-check';
import { afterAll, beforeAll, describe, expect, it } from 'vitest';

import { NodeTokenCache, type CachedTokenProfile } from './index.js';

let directory = '';

beforeAll(async () => {
  directory = await mkdtemp(join(tmpdir(), 'baukit-auth-node-doc-'));
});

afterAll(async () => {
  await rm(directory, { recursive: true });
});

const text = fc.string({ minLength: 1, maxLength: 12 });
const profileName = fc.stringMatching(/^[A-Za-z0-9][A-Za-z0-9._-]{0,15}$/u);
const profile: fc.Arbitrary<CachedTokenProfile> = fc.record(
  {
    accessToken: text,
    refreshToken: text,
    idToken: text,
    tokenType: text,
    issuedAt: fc.integer(),
    expiresAt: fc.integer({ min: 1 }),
    issuer: text,
    clientId: text,
    scopes: fc.array(text, { maxLength: 4 }),
    audience: text,
  },
  {
    requiredKeys: [
      'accessToken',
      'tokenType',
      'issuedAt',
      'expiresAt',
      'issuer',
      'clientId',
      'scopes',
    ],
  },
);

let counter = 0;
function freshCache(): NodeTokenCache {
  counter += 1;
  return new NodeTokenCache({
    namespace: 'test',
    path: join(directory, `tokens-${String(counter)}.json`),
  });
}

describe('NodeTokenCache document parsing', () => {
  it('round-trips any valid profile set', async () => {
    await fc.assert(
      fc.asyncProperty(fc.dictionary(profileName, profile, { maxKeys: 4 }), async (profiles) => {
        const cache = freshCache();
        for (const [name, value] of Object.entries(profiles)) await cache.write(name, value);
        for (const [name, value] of Object.entries(profiles)) {
          await expect(cache.read(name)).resolves.toEqual(value);
        }
      }),
      { numRuns: 40 },
    );
  });

  it('never pollutes prototypes and only returns own, validated profiles', async () => {
    const hostile = fc.oneof(
      fc.constantFrom('__proto__', 'constructor', 'prototype', 'toString', 'hasOwnProperty'),
      profileName,
    );
    await fc.assert(
      fc.asyncProperty(
        fc.dictionary(hostile, fc.oneof(profile, fc.jsonValue()), { maxKeys: 4 }),
        hostile,
        async (profiles, lookup) => {
          const cache = freshCache();
          const raw = `{"version":1,"namespace":"test","profiles":{${Object.entries(profiles)
            .map(([name, value]) => `${JSON.stringify(name)}:${JSON.stringify(value)}`)
            .join(',')}}}`;
          await writeFile(cache.path, raw, { mode: 0o600 });
          const result = await cache.read(lookup).catch((cause: unknown) => cause);
          expect(Object.prototype).not.toHaveProperty('accessToken');
          expect(Object.prototype).not.toHaveProperty('polluted');
          if (result === undefined) return;
          if (result instanceof TypeError) {
            expect(/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/u.test(lookup)).toBe(false);
            return;
          }
          if (result instanceof Error) {
            expect(result).toMatchObject({ code: 'cache_corrupt' });
            return;
          }
          expect(Object.hasOwn(profiles, lookup)).toBe(true);
          expect(Object.getPrototypeOf(result)).toBe(Object.prototype);
        },
      ),
      { numRuns: 200 },
    );
  });
});
